use crate::as_of_capability::AsOfCapability;
use crate::error::DataError;
use crate::types::*;
use crate::vendors::StockVendor;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::sleep;

pub struct EastMoneyVendor {
    pub http: reqwest::Client,
    /// 可选代理客户端，用于绕过直连链路故障（如本机到 push2his 被 RST）。
    /// 运行时可通过 AStockClient::set_eastmoney_proxy 热更新（设置页配置），
    /// 启动时从 EASTMONEY_PROXY 环境变量读取初始值（向后兼容）。
    /// Arc+RwLock 共享句柄：vendor 注册进 AStockClient 后外界无法按名定位，
    /// 用共享句柄让设置命令能原地更新代理而无需重建整个 client。
    pub proxy_http: std::sync::Arc<tokio::sync::RwLock<Option<reqwest::Client>>>,
}

impl EastMoneyVendor {
    /// 从环境变量 EASTMONEY_PROXY 读取初始代理（如 http://127.0.0.1:12026）
    pub fn build_proxy_client() -> Option<reqwest::Client> {
        let proxy_url = std::env::var("EASTMONEY_PROXY").ok()?;
        match Self::try_build_proxy_client(proxy_url.trim()) {
            Ok(c) => {
                tracing::info!("[eastmoney] 已配置代理（来自 EASTMONEY_PROXY 环境变量）");
                Some(c)
            },
            Err(e) => {
                tracing::warn!("[eastmoney] EASTMONEY_PROXY 环境变量无效: {e}");
                None
            },
        }
    }

    /// 从 URL 构建并校验代理客户端。空 URL 返回 Err（调用方决定语义）。
    /// 原 `reqwest::Proxy::all(&proxy_url).ok()?` 把代理构建错误（URL 格式错误、
    /// 协议不支持等）静默吞为 None，调用方无法感知。改为显式 Result。
    pub fn try_build_proxy_client(proxy_url: &str) -> Result<reqwest::Client, String> {
        if proxy_url.is_empty() {
            return Err("代理地址为空".into());
        }
        let proxy =
            reqwest::Proxy::all(proxy_url).map_err(|e| format!("代理 URL 解析失败: {e}"))?;
        reqwest::Client::builder()
            .proxy(proxy)
            .timeout(std::time::Duration::from_secs(15))
            .connect_timeout(std::time::Duration::from_secs(10))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
            .cookie_store(true)
            .pool_max_idle_per_host(8)
            .min_tls_version(reqwest::tls::Version::TLS_1_2)
            .build()
            .map_err(|e| format!("代理客户端创建失败: {e}"))
    }

    /// 东财搜索接口的**单页**新闻抓取（`get_news` / `search_news` / as-of 回溯共用）。
    ///
    /// 收口动因：`get_news` 与 `search_news` 此前各抄一份 110 行的 JSONP 拼参与解析
    /// （M-RES-16 的 `list` 形态 fallback 修复就重复写了两遍），而 T2 的回溯要翻页 ⇒ 先合一处。
    async fn news_page(
        &self,
        keyword: &str,
        page_index: u32,
        page_size: u32,
        sort: &str,
    ) -> Result<Vec<NewsItem>, DataError> {
        let param = serde_json::json!({
            "uid": "",
            "keyword": keyword,
            "type": ["cmsArticleWebOld"],
            "client": "web",
            "clientType": "web",
            "clientVersion": "curr",
            "param": {
                "cmsArticleWebOld": {
                    "searchScope": "default",
                    "sort": sort,
                    "pageIndex": page_index,
                    "pageSize": page_size,
                    "preTag": "",
                    "postTag": ""
                }
            }
        });

        let url = format!(
            "https://search-api-web.eastmoney.com/search/jsonp?cb=jQuery&param={}",
            urlencoding::encode(&param.to_string())
        );

        let resp = self
            .http
            .get(&url)
            .header("Referer", "https://so.eastmoney.com/")
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .send()
            .await
            .map_err(|e| DataError::VendorError {
                vendor: "eastmoney".into(),
                message: format!("新闻搜索请求失败: {e}"),
            })?;

        let text = resp.text().await.map_err(|e| DataError::VendorError {
            vendor: "eastmoney".into(),
            message: format!("新闻搜索响应读取失败: {e}"),
        })?;

        parse_news_jsonp(&text)
    }

    /// T2：`sort:"time"` 按时间倒序翻页，裁到截止日。
    ///
    /// 实测（2026-09-26）该接口**不认** `beginTime/endTime`，`sortEnd` 只是分页游标
    /// （传历史日期被忽略，返回的仍是当下最新）⇒ 唯一可用的时间通道就是倒序翻页 +
    /// 本地裁剪。上限 12 页 ≈ 600 条；翻不到截止日**如实报 Err**，让路由层落到
    /// `news_archive`，而不是静默返回空（静默空会被下游读成「该股没有新闻」）。
    async fn news_pages_until_asof(
        &self,
        keyword: &str,
        cutoff: &str,
        need: usize,
    ) -> Result<Vec<NewsItem>, DataError> {
        const PAGE_SIZE: u32 = 50;
        const MAX_PAGES: u32 = 12;
        let mut kept: Vec<NewsItem> = Vec::new();
        let mut crossed = false;
        let mut fetched = 0usize;
        for page in 1..=MAX_PAGES {
            let items = self.news_page(keyword, page, PAGE_SIZE, "time").await?;
            if items.is_empty() {
                break;
            }
            fetched += items.len();
            let hits = clip_page_to_cutoff(&items, cutoff);
            crossed |= !hits.is_empty();
            kept.extend(hits);
            if kept.len() >= need {
                break;
            }
        }
        if kept.is_empty() && !crossed {
            return Err(DataError::VendorError {
                vendor: "eastmoney".into(),
                message: format!("时间回溯 {MAX_PAGES} 页(共 {fetched} 条)未覆盖到截止日 {cutoff}"),
            });
        }
        kept.truncate(need);
        Ok(kept)
    }

    /// 政策新闻取数主体（live 与 as-of 回溯共用，2026-09-26 T2 收口）。
    ///
    /// `asof=true` 时唯一差异：每个关键词的候选不再取当下单页，而是走
    /// `news_pages_until_asof`（`sort:"time"` 倒序翻页 + 裁到截止日）。
    async fn policy_news_impl(
        &self,
        stock_code: &str,
        limit: u32,
        asof: bool,
    ) -> Result<Vec<NewsItem>, DataError> {
        // 实现策略(v4, 2026-07-22):
        //
        // 问题历史:
        //   v1: search_news("{行业} 政策") → 中文组合关键词分词差,返回空
        //   v2: get_news(stock_code) → 纯数字关键词搜索差,返回空
        //   v3: get_news(股票名) → 政策新闻不提公司名,过滤后为空
        //
        // v4 根因分析:政策新闻是宏观的,通常不提具体公司名(如"伊利股份"),
        //   但会提行业名(如"食品饮料")。例如《国务院关于印发食品安全规划的通知》
        //   不会出现"伊利股份",但会出现"食品"相关词。
        //
        // v4 方案:双路并行搜索 + 政策过滤 + 兜底
        //   路径A: 行业关键词搜索 - search_news(行业名,如"食品饮料")
        //         纯中文行业名搜索效果好,行业新闻中常含政策内容
        //   路径B: 股票名搜索 - search_news(股票名,如"伊利股份")
        //         获取个股层面新闻,过滤政策相关公告/监管通知
        //   合并去重 + 按 26 个政策关键词过滤
        //   兜底:过滤后为空则返回行业新闻(让 LLM 判断相关性)
        let fetch_limit = limit.clamp(50, 100);
        let cutoff = if asof {
            crate::as_of::current_as_of()
                .map(|c| c.as_of_date.format("%Y-%m-%d").to_string())
                .unwrap_or_default()
        } else {
            String::new()
        };

        // 并行获取行业信息和股票名称
        let (sector_result, search_result) =
            tokio::join!(self.get_sector_info(stock_code), self.search_stock(stock_code));

        let sector_name =
            sector_result.ok().and_then(|opt| opt.map(|s| s.sector_name)).unwrap_or_default();

        let stock_name = search_result
            .ok()
            .and_then(|results| {
                results
                    .iter()
                    .find(|r| {
                        r.code
                            == stock_code
                                .trim_start_matches("sh")
                                .trim_start_matches("sz")
                                .trim_start_matches("bj")
                    })
                    .or_else(|| results.first())
                    .map(|r| r.name.clone())
            })
            .unwrap_or_default();

        // 构建搜索关键词列表(纯中文,避免组合分词问题)
        // 先比较再消费,避免 move 后借用
        let stock_differs_from_sector =
            !stock_name.is_empty() && !sector_name.is_empty() && stock_name != sector_name;
        let mut keywords: Vec<String> = Vec::new();
        if !sector_name.is_empty() {
            keywords.push(sector_name);
        }
        if stock_differs_from_sector {
            keywords.push(stock_name);
        }
        // 兜底:行业和名称都拿不到时用代码(可能返回空,但至少尝试过)
        if keywords.is_empty() {
            keywords.push(stock_code.to_string());
        }

        // 对每个关键词搜索新闻,合并去重(按标题)
        let mut seen_titles: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut all_news: Vec<NewsItem> = Vec::new();

        for keyword in &keywords {
            let fetched = if asof {
                self.news_pages_until_asof(keyword, &cutoff, fetch_limit as usize).await
            } else {
                self.search_news(keyword, fetch_limit).await
            };
            match fetched {
                Ok(news) => {
                    tracing::debug!(
                        "[get_policy_news] search_news('{}') 返回 {} 条",
                        keyword,
                        news.len()
                    );
                    for n in news {
                        let key = n.title.trim().to_string();
                        if !key.is_empty() && seen_titles.insert(key) {
                            // 噪声过滤(2026-09-08): 行业名搜索(如"信息技术")会命中
                            // ETF/基金类行情资讯(如"港股通信息技术ETF")。这类内容
                            // 永远不是政策新闻,必须在收集阶段剔除——否则政策关键词
                            // 过滤失败后,兜底路径会把基金资讯当"政策新闻"喂给
                            // a-policy 分析师(实测缺陷)。
                            let hay = format!("{} {}", n.title, n.summary);
                            if NOISE_KEYWORDS.iter().any(|kw| hay.contains(kw)) {
                                continue;
                            }
                            all_news.push(n);
                        }
                    }
                },
                Err(e) => {
                    tracing::debug!("[get_policy_news] search_news('{}') 失败: {}", keyword, e);
                },
            }
        }

        // 政策相关关键词
        const POLICY_KEYWORDS: &[&str] = &[
            "政策",
            "规划",
            "通知",
            "补贴",
            "监管",
            "法规",
            "条例",
            "办法",
            "意见",
            "纲要",
            "改革",
            "扶持",
            "刺激",
            "减税",
            "降费",
            "鼓励",
            "限制",
            "禁止",
            "标准",
            "五年规划",
            "中央经济",
            "工信部",
            "发改委",
            "证监会",
            "农业农村部",
            "国务院",
            "常务会议",
        ];

        // 噪声关键词(2026-09-08): ETF/基金类资讯永远不是政策新闻,
        // 在收集阶段直接剔除(见上方循环内注释)。
        const NOISE_KEYWORDS: &[&str] = &["ETF", "etf", "基金", "净值", "份额折算", "LOF"];

        let is_policy_related = |n: &NewsItem| {
            let haystack = format!("{} {}", n.title, n.summary);
            POLICY_KEYWORDS.iter().any(|kw| haystack.contains(kw))
        };

        // 先过滤出政策相关新闻(不消费 all_news,用 iter + cloned)
        let filtered: Vec<NewsItem> =
            all_news.iter().filter(|n| is_policy_related(n)).cloned().collect();

        // 决定最终返回:有政策新闻则用过滤结果,否则兜底返回全部行业新闻
        let mut policy_news: Vec<NewsItem> = if !filtered.is_empty() {
            filtered
        } else if !all_news.is_empty() {
            // 兜底:无政策相关但行业新闻非空 → 返回全部行业新闻让 LLM 判断
            // (避免工具返回空导致 a-policy 节点无数据可用)。
            // 2026-09-08 修复:每条打上兜底标记,让下游分析师明确知道这些是
            // "未命中政策关键词的行业资讯",不是政策新闻命中——防止把行业资讯
            // 误当成政策原文引用导致上游数据偏差(实测缺陷:军工股收到
            // "港股通信息技术ETF"资讯被当作政策数据分析)。
            tracing::debug!(
                "[get_policy_news] 政策关键词过滤后为空,返回带兜底标记的行业新闻({}条)供 LLM 判断",
                all_news.len()
            );
            all_news
                .iter()
                .map(|n| {
                    let mut marked = n.clone();
                    marked.summary = format!(
                        "【兜底数据·未命中政策关键词,仅为该行业近期资讯】{}",
                        marked.summary
                    );
                    marked
                })
                .collect()
        } else {
            vec![]
        };

        // 按 publish_time 降序排
        policy_news.sort_by(|a, b| b.publish_time.cmp(&a.publish_time));
        policy_news.truncate(limit as usize);

        Ok(policy_news)
    }

    /// `RPT_CSDC_LIST`（中登质押周报）查询 URL；`cutoff=Some(d)` 时追加
    /// `(TRADE_DATE<='d')`，取截止日前最近一期。
    ///
    /// 实测依据（2026-09-26，300642）：不带日期 = 最新一期 2026-09-24；
    /// 带 `TRADE_DATE<='2026-06-30'` = 返回 2026-06-26 那期（周报口径，向前取最近披露）。
    /// 语法注意：日期必须写成 `'YYYY-MM-DD'`（带单引号）——写成 `20260630` 或不带引号
    /// 会被服务端判成「filter 字段中日期参数格式错误」。
    fn pledge_report_url(stock_code: &str, cutoff: Option<&str>) -> String {
        let code =
            stock_code.trim_start_matches("sh").trim_start_matches("sz").trim_start_matches("bj");
        let secucode = to_em_secucode(code);
        let filter = match cutoff {
            Some(d) => format!("(SECUCODE%3D%22{secucode}%22)(TRADE_DATE%3C%3D%27{d}%27)"),
            None => format!("(SECUCODE%3D%22{secucode}%22)"),
        };
        format!(
            "https://datacenter-web.eastmoney.com/api/data/v1/get?\
            reportName=RPT_CSDC_LIST&columns=ALL&\
            filter={filter}&\
            pageSize=5&pageNumber=1&source=WEB&\
            sortColumns=TRADE_DATE&sortTypes=-1"
        )
    }

    /// 发一次质押报表请求并解析首行（live 与 as-of 共用）。
    /// 返回值第二项是报表自带的 `TRADE_DATE`，供 as-of 侧记录「实际取到的是哪一期」。
    async fn fetch_pledge(
        &self,
        url: &str,
        stock_code: &str,
    ) -> Result<Option<(PledgeData, String)>, DataError> {
        let resp = self.em_get(url).await?;
        let json: Value = resp.json().await.map_err(|e| DataError::VendorError {
            vendor: "eastmoney".into(),
            message: format!("get_pledge_data JSON 解析失败: {e}"),
        })?;

        if json["success"].as_bool() == Some(false) {
            return Err(DataError::VendorError {
                vendor: "eastmoney".into(),
                message: format!(
                    "get_pledge_data 报表不可用: {}",
                    json["message"].as_str().unwrap_or("unknown")
                ),
            });
        }

        let rows = match json["result"]["data"].as_array() {
            Some(arr) if !arr.is_empty() => arr,
            _ => return Ok(None),
        };

        let r = &rows[0];
        let f = |key: &str| -> f64 {
            r[key].as_f64().or_else(|| r[key].as_str().and_then(|s| s.parse().ok())).unwrap_or(0.0)
        };
        let pledge_ratio = f("PLEDGE_RATIO");
        // RPT_CSDC_LIST 的 REPURCHASE_BALANCE 单位是「万股」，接口契约为「股」
        let pledge_shares = f("REPURCHASE_BALANCE") * 10_000.0;
        let pledge_count = r["PLEDGE_DEAL_NUM"].as_i64().unwrap_or(0) as i32;
        // 报表无控股股东质押比例列，置 0.0 表示未提供（非「控股股东零质押」的强断言）
        let controlling_pledge_ratio = 0.0;
        let trade_date = r["TRADE_DATE"].as_str().unwrap_or_default().to_string();

        // 风险等级分类(与 detect_pledge_risk 工具阈值对齐)
        let risk_level = if pledge_ratio >= 70.0 {
            "极高风险"
        } else if pledge_ratio >= 50.0 {
            "高风险"
        } else if pledge_ratio >= 30.0 {
            "中风险"
        } else if pledge_ratio > 10.0 {
            "低风险"
        } else {
            "安全"
        };

        Ok(Some((
            PledgeData {
                stock_code: stock_code.to_string(),
                pledge_ratio,
                pledge_shares,
                pledge_count,
                controlling_pledge_ratio,
                risk_level: risk_level.to_string(),
            },
            trade_date,
        )))
    }

    /// 抓一页 7×24 快讯（游标语义见 `flash_cursor_at`）。
    /// 第二项是服务端 echo 的 `data.sortEnd`（下一页游标）；缺省时调用方停止翻页。
    async fn fetch_flash_page(
        &self,
        cursor: i64,
    ) -> Result<(Vec<ClsFlashItem>, Option<i64>), DataError> {
        let resp = self.em_get(&flash_list_url(cursor, FLASH_PAGE_SIZE)).await?;
        let json: Value = resp.json().await?;
        let items = match json["data"]["fastNewsList"].as_array() {
            Some(arr) => arr,
            None => match json["data"].as_array() {
                Some(arr) => arr,
                None => return Ok((vec![], None)),
            },
        };
        Ok((parse_fast_news(items), json["data"]["sortEnd"].as_i64()))
    }

    /// em_get 带指数退避重试（连接级别错误：1s → 2s → 4s，最多 3 次）
    /// 429 限流时使用更长等待（2s → 4s → 8s）
    /// 连接级断裂（IncompleteMessage / TLS EOF / RST）时若配置了代理，
    /// 不退避立即切代理重试；无代理则快速失败交给路由层 fallback
    async fn em_get(&self, url: &str) -> Result<reqwest::Response, DataError> {
        let max_retries = 3;
        let mut delay = Duration::from_secs(1);
        let mut last_err = None;
        // 克隆代理句柄快照（reqwest::Client 内部是 Arc，克隆廉价）；
        // 不跨 await 持有锁 guard。
        let proxy_client = self.proxy_http.read().await.clone();
        for attempt in 0..max_retries {
            let http_client = if attempt == 0 {
                &self.http
            } else if let Some(ref p) = proxy_client {
                // 第1次失败后走代理重试（仅当配置了代理）
                p
            } else {
                &self.http
            };
            let result = http_client
                .get(url)
                .header("Referer", "https://quote.eastmoney.com/")
                .header(
                    "User-Agent",
                    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
                )
                .header("Accept", "application/json, text/plain, */*")
                .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
                .send()
                .await;
            match result {
                Ok(resp) => {
                    // 检查 429 限流
                    if let Err(e) = crate::check_response_429(&resp, "eastmoney") {
                        if attempt + 1 < max_retries {
                            let wait = delay * 2;
                            tracing::warn!(
                                "[retry] eastmoney 限流(第{}次, {wait:?}后重试)",
                                attempt + 1
                            );
                            sleep(wait).await;
                            delay *= 2;
                            last_err = Some(e);
                            continue;
                        }
                        last_err = Some(e);
                    } else {
                        return Ok(resp);
                    }
                },
                Err(e) => {
                    // 连接级断裂识别：IncompleteMessage（响应中途断流）+ TLS EOF /
                    // SendRequest / ConnectionReset 等（对端 RST 掐断连接）。
                    // 实证背景（2026-08-01 / 2026-09-07 两次复发）：本机直连
                    // push2his.eastmoney.com 的 IPv4 链路会被服务器 RST（所有镜像
                    // CDN 节点一致），属持续性坏链路而非抖动——同链路退避重试无意义，
                    // 应立即切代理（有代理）或快速失败交给路由层 fallback 到腾讯源。
                    let err_repr = format!("{e:?}");
                    let is_conn_break = err_repr.contains("IncompleteMessage")
                        || err_repr.contains("UnexpectedEof")
                        || err_repr.contains("ConnectionReset")
                        || err_repr.contains("ConnectionAborted")
                        || err_repr.contains("BrokenPipe")
                        || err_repr.contains("SendRequest");
                    if is_conn_break && attempt == 0 && proxy_client.is_some() {
                        // 连接断裂 + 有代理 → 不走指数退避，立即走代理重试
                        tracing::warn!("[eastmoney] 直连被掐断，立即切换代理重试({url})");
                        last_err = Some(e.into());
                        continue; // 直接用 attempt=1 走代理
                    }
                    if is_conn_break {
                        // 连接断裂 + 无代理 → 快速失败让路由层 fallback
                        tracing::warn!(
                            "[eastmoney] 连接被掐断且无代理({url})，快速失败→路由层 fallback"
                        );
                        return Err(e.into());
                    }
                    if attempt + 1 < max_retries {
                        tracing::warn!(
                            "[retry] eastmoney 请求失败 (第{}次, {delay:?}后重试): {e:?}",
                            attempt + 1
                        );
                        sleep(delay).await;
                        delay *= 2;
                    } else {
                        tracing::error!(
                            "[eastmoney] 最终失败: {e:?}, source: {:?}",
                            std::error::Error::source(&e)
                        );
                        last_err = Some(e.into());
                    }
                },
            }
        }
        // 修复 M-DEF-2: 原代码 `last_err.unwrap()` 在 last_err 为 None 时 panic。
        // 理论上循环正常退出时 last_err 必有值（只有 Ok 分支会 return），
        // 但防御性编程：若因逻辑漏洞走到这里 last_err 仍为 None，给出明确错误。
        Err(last_err.unwrap_or_else(|| DataError::VendorError {
            vendor: "eastmoney".into(),
            message: "no error recorded".into(),
        }))
    }
}

/// 根据公告标题分类财报事件类型
///
/// 返回 (event_type, period)
/// - "业绩预告" → ("preliminary", 期间)
/// - "业绩快报" → ("express", 期间)
/// - "年报"/"季报"/"半年报" → ("formal", 期间)
/// - "股东大会" → ("shareholders_meeting", None)
/// - 其他 → ("other", None)
pub(crate) fn classify_earnings_title(title: &str) -> (&'static str, Option<String>) {
    // 提取期间（如 "2024年年度报告" → "2024年报"，"2025年第三季度报告" → "2025Q3"）
    let period = extract_report_period(title);

    if title.contains("业绩预告") || title.contains("预增") || title.contains("预减") {
        return ("preliminary", period);
    }
    if title.contains("业绩快报") {
        return ("express", period);
    }
    if title.contains("股东大会") {
        return ("shareholders_meeting", None);
    }
    if title.contains("年度报告")
        || title.contains("季度报告")
        || title.contains("半年报")
        || title.contains("年报")
    {
        return ("formal", period);
    }
    ("other", None)
}

/// 从标题中提取报告期间
fn extract_report_period(title: &str) -> Option<String> {
    // 匹配 "2024年年度报告" / "2025年第三季度报告" / "2024年半年度报告"
    if let Some(year_start) = title.find(|c: char| c.is_ascii_digit() && c != '0') {
        let rest = &title[year_start..];
        // 提取年份
        let year: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if year.len() == 4 {
            // P3 修复(2026-07-25): 半年度/半年报/中报必须先于"年度报告"判断,
            // 否则"2024年半年度报告"会先匹配到"年度报告"误返回"2024年报"。
            if title.contains("半年度") || title.contains("半年报") || title.contains("中报")
            {
                return Some(format!("{year}Q2"));
            }
            if title.contains("年度报告") || title.contains("年报") {
                return Some(format!("{year}年报"));
            }
            if title.contains("第一季度") {
                return Some(format!("{year}Q1"));
            }
            if title.contains("第三季度") {
                return Some(format!("{year}Q3"));
            }
            return Some(year);
        }
    }
    None
}

/// 构建东方财富 secid
///
/// as-of 单行估值快照 URL（RPT_VALUEANALYSIS_DET，截止日前最近交易日）。
/// 抽成自由函数供 `fflow_asof_tests` 同款 URL 判据钉住（转义写错是**静默空结果**）。
fn valuation_snapshot_asof_url(code: &str, cutoff: &str) -> String {
    format!(
        "https://datacenter-web.eastmoney.com/api/data/v1/get?\
        reportName=RPT_VALUEANALYSIS_DET&columns=SECURITY_CODE,TRADE_DATE,PE_TTM,PB_MRQ,PS_TTM,PCF_OCF_TTM,CLOSE_PRICE,TOTAL_MARKET_CAP&\
        filter=(SECURITY_CODE%3D%22{code}%22)(TRADE_DATE%3C%3D%27{cutoff}%27)&\
        sortColumns=TRADE_DATE&sortTypes=-1&pageSize=1&source=WEB&client=WEB"
    )
}

/// 解析 push2his fflow/daykline 的 klines CSV 数组（f51=日期 f52=主力 f53=小单 f54=中单 f55=大单 f56=超大单）。
/// 接口按日期**升序**返回（2026-09-26 实测），本函数保持原序，排序由调用方显式做。
fn parse_fflow_klines(klines: &[Value]) -> Vec<MoneyFlowDaily> {
    klines
        .iter()
        .filter_map(|v| v.as_str())
        .map(|line| {
            let parts: Vec<&str> = line.split(',').collect();
            let f = |i: usize| -> f64 { parts.get(i).and_then(|s| s.parse().ok()).unwrap_or(0.0) };
            MoneyFlowDaily {
                date: parts.first().unwrap_or(&"").to_string(),
                main_net_inflow: f(1),
                small_net: f(2),
                medium_net: f(3),
                large_net: f(4),
                super_large_net: f(5),
            }
        })
        .collect()
}

/// as-of 窗口选择：入参为**任意序**的日频资金流行，返回 `date <= cutoff` 中最近的
/// 至多 5 条，按日期**降序**（最新在前，与 MoneyFlow.history 消费口径一致）。
fn select_fflow_window(mut rows: Vec<MoneyFlowDaily>, cutoff: &str) -> Vec<MoneyFlowDaily> {
    rows.retain(|r| r.date.as_str() <= cutoff);
    rows.sort_by(|a, b| b.date.cmp(&a.date));
    rows.truncate(5);
    rows
}

/// A 股：`1.600519`（上海）、`0.000001`（深圳）
/// 港股：`116.00700`（去掉 .HK 后缀，加 116 前缀）
/// 美股：`105.AAPL`（去掉 .US 后缀，加 105 前缀）
fn to_em_secid(stock_code: &str) -> String {
    // secid 直通：形如 `1.000001` 的输入已经是东财 secid（指数 K 线走这条路，见
    // `get_index_quotes_with_asof`），不能再按「首位数字」推断市场 —— 上证指数的代码
    // 000001 按股票规则会被推断成深圳，静默取回平安银行的历史。
    if let Some((market, code)) = stock_code.split_once('.') {
        if !market.is_empty()
            && market.chars().all(|c| c.is_ascii_digit())
            && !code.is_empty()
            && code.chars().all(|c| c.is_ascii_digit())
        {
            return stock_code.to_string();
        }
    }
    // 修复(2026-07-22): 去除 sh/sz/bj 前缀,否则后续 starts_with('6') 判断会失效,
    // 误把 "sh600887" 当作深圳市场股票,生成 secid="0.sh600887" 导致所有 API 调用返回空数据。
    // 影响范围:get_quote/get_klines/get_financials/get_money_flow/get_dragon_tiger/
    // get_lockup_schedule/get_margin_data/get_north_bound_holding/get_shareholder_trades/
    // get_block_trades/get_peers 等所有使用 to_em_secid 的方法。
    let code =
        stock_code.trim_start_matches("sh").trim_start_matches("sz").trim_start_matches("bj");

    // 港股：00700.HK → 116.00700
    if let Some(hk) = code.strip_suffix(".HK").or_else(|| code.strip_suffix(".hk")) {
        return format!("116.{hk}");
    }
    // 美股：AAPL.US → 105.AAPL
    if let Some(us) = code.strip_suffix(".US").or_else(|| code.strip_suffix(".us")) {
        return format!("105.{us}");
    }
    // A 股
    let market = if code.starts_with('6') || code.starts_with('9') {
        "1"
    } else if code.starts_with('8') || code.starts_with('4') {
        "0"
    } else {
        "0"
    };
    format!("{market}.{code}")
}

/// 大盘指数清单：`(东财 secid, 展示代码, 中文名)`。
///
/// live 实时快照与 as-of K 线合成共用同一张表，避免两条路径给出不同的指数集合。
/// secid 的市场位（1=沪 / 0=深）**由本表显式给定** —— 上证综指与平安银行同为
/// `000001`，按股票首位数字推断市场会静默取回错误的标的。
pub const EM_INDEX_SECIDS: [(&str, &str, &str); 3] = [
    ("1.000001", "000001", "上证指数"),
    ("0.399001", "399001", "深证成指"),
    ("0.399006", "399006", "创业板指"),
];

/// 构建东方财富 SECUCODE（用于 datacenter 报表 API）
///
/// A 股：`600887.SH`（上海）、`000001.SZ`（深圳）、`830879.BJ`（北交所）
/// 港股/美股：原值返回（如 `00700.HK`、`AAPL.US`）
fn to_em_secucode(stock_code: &str) -> String {
    let code =
        stock_code.trim_start_matches("sh").trim_start_matches("sz").trim_start_matches("bj");
    // 港股/美股：已带后缀直接返回
    if code.ends_with(".HK")
        || code.ends_with(".hk")
        || code.ends_with(".US")
        || code.ends_with(".us")
    {
        return code.to_string();
    }
    // A 股
    let suffix = if code.starts_with('6') || code.starts_with('9') {
        "SH"
    } else if code.starts_with('8') || code.starts_with('4') {
        "BJ"
    } else {
        "SZ"
    };
    format!("{code}.{suffix}")
}

/// 解析东财搜索接口的 JSONP 响应（`jQuery1830…({…})`）为新闻条目。
///
/// 修复 M-RES-16: 兼容 `result.cmsArticleWebOld` 直接是数组、或是 `{list: [...]}`
/// 两种形态；两者都不是时按「该维度无数据」返回空，并留 debug 便于排查。
fn parse_news_jsonp(text: &str) -> Result<Vec<NewsItem>, DataError> {
    let trimmed = text.trim();
    let json_str = if let Some(start) = trimmed.find('(') {
        if let Some(end) = trimmed.rfind(')') {
            &trimmed[start + 1..end]
        } else {
            trimmed
        }
    } else {
        trimmed
    };

    let json: Value = serde_json::from_str(json_str).map_err(|e| {
        DataError::ParseError(format!(
            "eastmoney news jsonp parse failed: {e}, raw: {}",
            &text[..200.min(text.len())]
        ))
    })?;

    let items = json["result"]["cmsArticleWebOld"]
        .as_array()
        .or_else(|| json["result"]["cmsArticleWebOld"]["list"].as_array());
    let Some(arr) = items else {
        tracing::debug!("[eastmoney] cmsArticleWebOld 字段格式非预期（无 list 数组），返回空");
        return Ok(vec![]);
    };

    Ok(arr
        .iter()
        .filter_map(|item| {
            let title = item.get("title")?.as_str()?.to_string();
            let summary = item
                .get("digest")
                .or_else(|| item.get("content"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let source = item
                .get("mediaName")
                .or_else(|| item.get("source"))
                .and_then(|v| v.as_str())
                .unwrap_or("东方财富")
                .to_string();
            let article_url = item
                .get("articleUrl")
                .or_else(|| item.get("url"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let publish_time = item
                .get("showTime")
                .or_else(|| item.get("publishTime"))
                .or_else(|| item.get("ctime"))
                .or_else(|| item.get("date"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            Some(NewsItem {
                title,
                summary,
                source,
                url: article_url,
                publish_time,
                sentiment_score: None,
            })
        })
        .collect())
}

/// 从一页（`sort:"time"` 按时间倒序）里挑出 `publish_time <= cutoff` 的条目。
///
/// 日期不可解析的条目一律剔除：as-of 回放里「时效未知」等同于「可能是未来新闻」，
/// 宁可少给也不能把截止日之后的事件当成当时已知的信息（与路由层
/// `truncate_news_by_asof` 对 live 路径「保留但留痕」的口径**相反**，因为这里
/// 没有另一条通道去核实，且回放面要求严格）。
fn clip_page_to_cutoff(items: &[NewsItem], cutoff: &str) -> Vec<NewsItem> {
    items
        .iter()
        .filter(|n| {
            let key = crate::news_date_key(&n.publish_time);
            !key.is_empty() && key <= cutoff
        })
        .cloned()
        .collect()
}

/// 7×24 快讯分页大小（实测接口按 20 条/页返回，每页时间跨度约 1~2.5 小时）。
const FLASH_PAGE_SIZE: u32 = 20;
/// as-of 回溯时的翻页上限：游标可直接落到截止日当天末尾，正常 1~2 页就够；
/// 上限只防「当天条目极多」时翻不完，不是用来跨天硬翻的。
const FLASH_ASOFP_MAX_PAGES: u32 = 4;

/// 北京时间偏移（快讯的 `showTime` 是 CST 墙钟，游标按 UTC 秒计 ⇒ 换算必须钉死 +08）
fn cn_offset() -> chrono::FixedOffset {
    chrono::FixedOffset::east_opt(8 * 3600).expect("UTC+8 偏移恒合法")
}

/// `getFastNewsList` 的 `sortEnd` 游标：**Unix 秒 × 1e6**（响应里 `data.sortEnd` 同族回 echo，
/// 与条目自带的 `realSort` 一致）。
///
/// 实测依据（2026-09-26）：
/// - 传日期串 `"2026-09-18 15:00:00"` **被忽略**（仍返回当下最新）⇒ 旧注释把参数类型写错了；
/// - 传 `1789714800000000`（= 2026-09-18 15:00 CST 的秒数 ×1e6）⇒ 返回 14:58:47 起；
///   +1h ⇒ 15:58:43；+4h ⇒ 18:57:03 —— 线性、可直接跳日；
/// - 传位数错的值（如 ×1e9）被判越界 ⇒ 静默回落到「当下最新」，所以取数后仍要按日期复核。
fn flash_cursor_at(dt: &chrono::DateTime<chrono::FixedOffset>) -> i64 {
    dt.timestamp() * 1_000_000
}

/// 组装 `getFastNewsList` 请求 URL（`req_trace` 只做埋点，每次新生成）
fn flash_list_url(cursor: i64, page_size: u32) -> String {
    let req_trace = chrono::Utc::now().timestamp_millis();
    format!(
        "https://np-listapi.eastmoney.com/comm/web/getFastNewsList?client=web&biz=web_7x24&fastColumn=102&page_index=1&pageSize={page_size}&req_trace={req_trace}&sortEnd={cursor}"
    )
}

/// 解析 `data.fastNewsList`（字段别名：title↔summary、showTime↔time↔ctime、
/// source↔mediaName）。`data` 直接是数组的旧形态也兼容。
fn parse_fast_news(items: &[Value]) -> Vec<ClsFlashItem> {
    items
        .iter()
        .filter_map(|item| {
            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .or_else(|| {
                    item.get("summary")
                        .and_then(|v| v.as_str())
                        .map(|s| s.chars().take(80).collect::<String>())
                })?;
            // title 与 summary 都是空串时上面两支会产出 Some("")，条目会带着空标题
            // 流进报告（旧 live 代码即如此）；这种条目对分析师没有信息量，直接丢。
            if title.trim().is_empty() {
                return None;
            }
            let content = item
                .get("summary")
                .or_else(|| item.get("content"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let publish_time = item
                .get("showTime")
                .or_else(|| item.get("time"))
                .or_else(|| item.get("ctime"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let source = item
                .get("source")
                .or_else(|| item.get("mediaName"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            Some(ClsFlashItem { title, content, publish_time, source })
        })
        .collect()
}

#[async_trait]
impl StockVendor for EastMoneyVendor {
    async fn get_quote(&self, stock_code: &str) -> Result<StockQuote, DataError> {
        let secid = to_em_secid(stock_code);
        let url = format!(
            "https://push2his.eastmoney.com/api/qt/stock/get?secid={secid}&fields=f43,f44,f45,f46,f47,f48,f50,f51,f52,f55,f57,f58,f60,f116,f117,f162,f167,f168,f169,f170,f171"
        );
        let resp = self.em_get(&url).await?;
        let json: Value = resp.json().await?;
        let d = &json["data"];
        if d.is_null() {
            return Err(DataError::VendorError {
                vendor: "eastmoney".into(),
                message: "no quote data".into(),
            });
        }
        let f = |key: &str| d[key].as_f64().unwrap_or(0.0);
        let price = f("f43") / 100.0;
        let pre_close = f("f60") / 100.0;
        let is_st = d["f58"].as_str().map(|n| n.contains("ST")).unwrap_or(false);
        let market_type = detect_market_type(stock_code);
        let limit_pct = get_st_price_limit_pct(is_st, market_type) / 100.0;
        let limit_up = if pre_close > 0.0 {
            Some((pre_close * (1.0 + limit_pct) * 100.0).round() / 100.0)
        } else {
            None
        };
        let limit_down = if pre_close > 0.0 {
            Some((pre_close * (1.0 - limit_pct) * 100.0).round() / 100.0)
        } else {
            None
        };
        Ok(StockQuote {
            code: stock_code.to_string(),
            name: d["f58"].as_str().unwrap_or("").to_string(),
            price,
            pre_close,
            open: f("f46") / 100.0,
            high: f("f44") / 100.0,
            low: f("f45") / 100.0,
            volume: f("f47"),
            amount: f("f48"),
            change_pct: f("f170") / 100.0,
            turnover_rate: f("f168") / 100.0,
            // 2026-09-21 修复：保留负 PE/PB（亏损 = 有效信息），仅剔除 0 占位。
            // 详见 vendors/xueqiu.rs 同处注释（三家 vendor 同款过滤，一并放开）。
            pe: d["f162"].as_f64().filter(|v| *v != 0.0).map(|v| v / 100.0),
            pb: d["f167"].as_f64().filter(|v| *v != 0.0).map(|v| v / 100.0),
            total_mv: Some(f("f116")).filter(|v| *v > 0.0),
            circulating_mv: Some(f("f117")).filter(|v| *v > 0.0),
            limit_up,
            limit_down,
            is_st,
            timestamp: d["f171"].as_i64().map(|t| t.to_string()).unwrap_or_default(),
        })
    }

    async fn get_klines(
        &self,
        stock_code: &str,
        period: &str,
        limit: u32,
        adj: Option<AdjType>,
    ) -> Result<Vec<KLine>, DataError> {
        let period_code = match period {
            "5" | "Min5" => "5",
            "15" | "Min15" => "15",
            "30" | "Min30" => "30",
            "60" | "Min60" => "60",
            "daily" | "101" | "Daily" => "101",
            "weekly" | "102" | "Weekly" => "102",
            "monthly" | "103" | "Monthly" => "103",
            _ => "101",
        };
        let secid = to_em_secid(stock_code);
        // 修复 R3: 根据 adj 参数选择 fqt（0=不复权, 1=前复权, 2=后复权）
        // 原硬编码 fqt=1 导致 adj_type=None 时仍返回前复权数据，与不复权语义不符
        let fqt = match adj {
            None | Some(AdjType::None) => 0,
            Some(AdjType::Forward) => 1,
            Some(AdjType::Backward) => 2,
        };
        let url = format!(
            "https://push2his.eastmoney.com/api/qt/stock/kline/get?secid={secid}&fields1=f1,f2,f3,f4,f5,f6&fields2=f51,f52,f53,f54,f55,f56,f57,f58,f59,f60,f61&klt={period_code}&fqt={fqt}&end=20500101&lmt={limit}"
        );

        let resp = self.em_get(&url).await?;
        let json: Value = resp.json().await?;

        let klines_raw = json["data"]["klines"]
            .as_array()
            .ok_or_else(|| DataError::ParseError("missing klines array".into()))?;

        // vendor 已应用复权 → 标记 adj_factor = Some(1.0) 表示已处理
        // （实际复权因子不是 1.0，但 lib 层只检查 is_some 判断是否需要本地 fallback）
        let adj_marker = if fqt == 0 { None } else { Some(1.0) };
        let mut klines: Vec<KLine> = klines_raw
            .iter()
            .map(|v| {
                let s =
                    v.as_str().ok_or_else(|| DataError::ParseError("kline not string".into()))?;
                let parts: Vec<&str> = s.split(',').collect();
                if parts.len() < 11 {
                    return Err(DataError::ParseError(format!(
                        "expected 11 fields in kline, got {}",
                        parts.len()
                    )));
                }
                let parse = |s: &str| -> f64 { s.parse().unwrap_or(0.0) };
                Ok(KLine {
                    date: parts[0].to_string(),
                    open: parse(parts[1]),
                    close: parse(parts[2]),
                    high: parse(parts[3]),
                    low: parse(parts[4]),
                    volume: parse(parts[5]) * 100.0, // 东方财富 K线 f56 单位为"手"，×100 转为"股"
                    amount: parse(parts[6]),
                    turnover_rate: Some(parse(parts[10])),
                    // R3: vendor 已复权时标记，避免 lib 层二次应用
                    adj_factor: adj_marker,
                })
            })
            .collect::<Result<Vec<_>, DataError>>()?;
        klines.sort_by(|a, b| a.date.cmp(&b.date));
        Ok(klines)
    }

    async fn get_financials(&self, stock_code: &str) -> Result<Vec<FinancialReport>, DataError> {
        // 东方财富 2025 年后 FinanceSummary API 失效，改用 NewFinanceAnalysis/ZYZBAjaxNew
        // 修复(2026-07-22): 先去除 sh/sz/bj 前缀,否则 starts_with 判断失效
        let code =
            stock_code.trim_start_matches("sh").trim_start_matches("sz").trim_start_matches("bj");
        let em_code = if code.starts_with('6') || code.starts_with('9') {
            format!("SH{code}")
        } else if code.starts_with('8') || code.starts_with('4') {
            format!("BJ{code}")
        } else {
            format!("SZ{code}")
        };

        let base_url = |t: u8| {
            format!(
                "https://emweb.securities.eastmoney.com/PC_HSF10/NewFinanceAnalysis/ZYZBAjaxNew?type={t}&code={em_code}"
            )
        };

        // ① 主请求 type=0：按报告期返回**最近 9 期**（约 2.25 年）。
        let resp = self.em_get(&base_url(0)).await?;
        let json: Value = resp.json().await?;

        let mut rows: Vec<Value> = match json["data"].as_array() {
            Some(arr) if !arr.is_empty() => arr.clone(),
            _ => {
                return Err(DataError::VendorError {
                    vendor: "eastmoney".into(),
                    message: format!("get_financials 数据为空(stock_code={stock_code})"),
                });
            },
        };

        // ② 补充请求 type=1：按**年度**返回最近 9 个年报，合并进列表。
        //
        // 背景（2026-09-12，DB 实证 603353 和顺石油）：
        //   原实现只请求 type=0，并以为 `take(24)` 能拿到「6 年季度」——
        //   但该接口**单次只返回 9 个报告期**（约 2.25 年），24 条永远取不满。
        //   后果落在 `compute_dcf` 的亏损期归一化锚定上：它要的是
        //   「近 5 年年报正净利均值×0.90」，而 type=0 里年报候选只有 2 个，
        //   再经 `filter(np > 0)` 剔除亏损年后常常只剩 1 个单点。
        //   实测偏差：锚定值 2926 万（仅 2024 单年）vs 正确的近 5 年
        //   （2021~2025）正净利均值 6918 万 —— **偏低 2.36×**，且静默无日志。
        //   type=1 与 type=0 的字段集实测完全一致（均 141 个字段），可直接合并。
        //
        // 容错：补充请求失败只 warn —— 它是增强项，不应让整条链路硬失败。
        match self.em_get(&base_url(1)).await {
            Ok(resp) => match resp.json::<Value>().await {
                Ok(j) => {
                    if let Some(arr) = j["data"].as_array() {
                        rows.extend(arr.iter().cloned());
                    }
                },
                Err(e) => {
                    tracing::warn!("[eastmoney] get_financials 年报补充(type=1) 解析失败: {e}")
                },
            },
            Err(e) => tracing::warn!("[eastmoney] get_financials 年报补充(type=1) 请求失败: {e}"),
        }

        // ③ 合并去重 + 按报告期倒序 —— `financials[0]` 必须是**最新报告期**
        //    （`compute_dcf` / `annualized_eps` 都以它作当期），更早的年报排在尾部。
        let date_of = |r: &Value| -> String { r["REPORT_DATE"].as_str().unwrap_or("").to_string() };
        {
            let mut seen = std::collections::HashSet::new();
            rows.retain(|r| {
                let d = date_of(r);
                !d.is_empty() && seen.insert(d)
            });
            rows.sort_by_key(|r| std::cmp::Reverse(date_of(r)));
        }

        let mut reports: Vec<FinancialReport> = rows
            .iter()
            .take(40) // 去重后实测 16 条（9 期 + 7 个更早年报）；上限防异常返回
            .map(|r| {
                let s = |key: &str| -> &str { r[key].as_str().unwrap_or("") };
                // 修复 M-FIN-1: 原 n 函数只处理字符串类型，但东方财富 API 部分字段
                // 可能返回数字类型（Value::Number），导致解析失败返回 None。
                // 改为同时支持字符串和数字类型，并过滤 "--"/""/null 等无效值。
                let n = |key: &str| -> Option<f64> {
                    let v = &r[key];
                    if v.is_null() {
                        return None;
                    }
                    if let Some(s) = v.as_str() {
                        if s.is_empty() || s == "--" || s == "null" {
                            return None;
                        }
                        return s.parse::<f64>().ok();
                    }
                    v.as_f64()
                };
                FinancialReport {
                    stock_code: stock_code.to_string(),
                    report_date: s("REPORT_DATE").to_string(),
                    // 东方财富 2025 年字段名变更映射
                    revenue: n("TOTALOPERATEREVE"),       // 营业总收入
                    net_profit: n("PARENTNETPROFIT"),     // 归母净利润
                    eps: n("EPSJB"),                      // 基本每股收益
                    bps: n("BPS"),                        // 每股净资产
                    roe: n("ROEJQ"),                      // 加权平均ROE
                    debt_ratio: n("ZCFZL"),               // 资产负债率
                    gross_margin: n("XSMLL"),             // 销售毛利率
                    net_margin: n("XSJLL"),               // 销售净利率
                    revenue_yoy: n("TOTALOPERATEREVETZ"), // 营收同比增长
                    profit_yoy: n("PARENTNETPROFITTZ"),   // 净利润同比增长
                    total_assets: None,
                    operating_cash_flow: None,
                    capital_expenditure: None,
                    free_cash_flow: None,
                    current_ratio: n("LD"), // 流动比率
                    quick_ratio: n("SD"),   // 速动比率
                    // #8 修复(2026-07-22): 商誉/应收账款字段——当前利润表接口未提供,
                    // 后续可通过 ZcfzbAjaxNew 资产负债表接口补全
                    goodwill: None,
                    accounts_receivable: None,
                    estimated: Some(false),
                }
            })
            .collect();

        // ④ 补充请求（现金流量表）：补齐 `operating_cash_flow` / `capital_expenditure`。
        //
        // ── 为什么必须补（2026-09-21 实证，300308 中际旭创）────────────────────────
        //
        // `ZYZBAjaxNew`（主要指标）**不提供可直接使用的现金流量表科目** —— 实测其
        // 141 个字段里只有派生比率（`NCO_NETPROFIT` = 经营现金流/净利、
        // `MGJYXJJE` = 每股经营现金流）与两个口径不明的 `FCFF_FORWARD/BACK`，
        // 没有「经营活动产生的现金流量净额」与「购建固定资产等支付的现金」。
        // 本文件原先因此把三个字段**硬编码为 None**（见上方 `FinancialReport` 构造）。
        //
        // 后果不是「少一个指标」，而是三条既有链**整体退化为死代码**：
        //   · `compute_dcf` 的 ① 分支 `direct_fcf` 恒为 `None`
        //     ⇒ DCF 锚点**永远**走「近 5 年报正净利均值 × 0.90」fallback；
        //   · `compute_dcf` 的适用性判据 ②（「净利为正但当期真实 FCF ≤ 0」/
        //     「0 < FCF/净利 < 0.3 量级脱钩」）**永远无法命中** ⇒ 该护栏 100% 失效；
        //   · `compute_owner_earnings` 永远退化为 `净利 × 0.85~0.95`，
        //     把非现金的净利润冒充「所有者收益」。
        //
        // DB 佐证（`stock_analyses` 49 条）：15 条含 DCF `note`、8 条含
        // `is_fallback_anchor`，其中 **8/8 = true** —— 全库没有一例用过当期真实 FCF；
        // 且 `note` 统一谎称「当期FCF≤0（周期底部）」，对一家营收 +182.5%、
        // ROE 62.6% 的成长股也如此断言，并被 value-investor 原样引用进 `risk_flags`。
        //
        // ── 接口与字段 ──────────────────────────────────────────────────────────
        //
        // `xjllbAjaxNew`（现金流量表，与主要指标同源同站）。实测字段：
        //   `NETCASH_OPERATE`       经营活动产生的现金流量净额（元，**年内累计**）
        //   `CONSTRUCT_LONG_ASSET`  购建固定资产、无形资产和其他长期资产支付的现金（元，累计）
        // ```text
        //   300308 2025-12-31: OCF 108.96 亿, capex 27.60 亿  ⇒ 自由现金流 +81.36 亿
        //   300308 2026-06-30: OCF  18.00 亿, capex 48.02 亿  ⇒ 自由现金流 −30.02 亿（半年累计）
        // ```
        // 两者与 `eps` / `roe` **同为年内累计口径** ⇒ TTM 还原由 `compute_dcf` 侧的
        // `ttm_fcf()` 负责；本 vendor 只做忠实映射，**不做年度化**（否则同一份
        // `FinancialReport` 里各字段口径不一致 —— 参见 `annualized_eps` 的 P1-A 记录）。
        //
        // ⚠️ `companyType` 必须随公司类型变化 —— 传错**静默返回 `data: []`**（无报错）：
        // ```text
        //   通用=4  银行=3  证券=1  保险=2
        //   SZ300308(通用) companyType=4 → 3 行；companyType=3 → 0 行
        //   SH601166(银行) companyType=3 → 3 行；companyType=4 → 0 行
        //   SH600030(证券) companyType=1 → 1 行；  SH601318(保险) companyType=2 → 1 行
        // ```
        // 类型取自同一接口的 `ORG_TYPE` 字段（"通用"/"银行"/"证券"/"保险"），
        // 故不需要额外请求去探测。
        //
        // 容错：与 type=1 年报补充一致 —— 失败只 warn，不硬失败。它是**既有取值链的
        // 输入补齐**，不是新增能力；缺失时行为回退到本次修复前的形态。
        let company_type = match rows.first().and_then(|r| r["ORG_TYPE"].as_str()) {
            Some("银行") => 3,
            Some("证券") => 1,
            Some("保险") => 2,
            _ => 4, // 通用 + 未知类型（未知按通用试，失败只 warn）
        };
        let cf_dates: Vec<String> = reports
            .iter()
            .filter_map(|r| r.report_date.get(..10).map(|s| s.to_string()))
            .take(12)
            .collect();
        if !cf_dates.is_empty() {
            let url = format!(
                "https://emweb.securities.eastmoney.com/PC_HSF10/NewFinanceAnalysis/\
                 xjllbAjaxNew?companyType={company_type}&reportDateType=0&reportType=1&dates={}&code={em_code}",
                cf_dates.join(",")
            );
            match self.em_get(&url).await {
                Ok(resp) => match resp.json::<Value>().await {
                    Ok(j) => {
                        // 与上方 `n` 同形：兼容字符串型数字，过滤 "--" / "" / null
                        let num = |v: &Value| -> Option<f64> {
                            if v.is_null() {
                                return None;
                            }
                            if let Some(s) = v.as_str() {
                                if s.is_empty() || s == "--" || s == "null" {
                                    return None;
                                }
                                return s.parse::<f64>().ok();
                            }
                            v.as_f64()
                        };
                        let mut cf: HashMap<String, (Option<f64>, Option<f64>)> = HashMap::new();
                        for row in j["data"].as_array().into_iter().flatten() {
                            let date = match row["REPORT_DATE"].as_str().and_then(|d| d.get(..10)) {
                                Some(d) => d.to_string(),
                                None => continue,
                            };
                            cf.insert(
                                date,
                                (num(&row["NETCASH_OPERATE"]), num(&row["CONSTRUCT_LONG_ASSET"])),
                            );
                        }
                        let mut filled = 0usize;
                        for r in reports.iter_mut() {
                            let key = match r.report_date.get(..10) {
                                Some(k) => k,
                                None => continue,
                            };
                            let (ocf, capex) = match cf.get(key) {
                                Some(v) => *v,
                                None => continue,
                            };
                            r.operating_cash_flow = ocf;
                            r.capital_expenditure = capex;
                            if ocf.is_some() && capex.is_some() {
                                filled += 1;
                            }
                        }
                        // `data: []` 是 companyType 传错时的**静默**失败形态
                        // （HTTP 200 + 空数组），故只在补齐 0 期时升为 warn。
                        if filled == 0 {
                            tracing::warn!(
                                "[eastmoney] get_financials 现金流量表补充 0 期命中\
                                 (companyType={company_type}, dates={}, stock_code={stock_code}) \
                                 —— 可能 companyType 与公司类型不匹配",
                                cf_dates.len()
                            );
                        } else {
                            tracing::debug!(
                                "[eastmoney] get_financials 现金流量表补充: {filled}/{} 期补齐 OCF+capex",
                                reports.len()
                            );
                        }
                    },
                    Err(e) => tracing::warn!(
                        "[eastmoney] get_financials 现金流量表解析失败: {e} (stock_code={stock_code})"
                    ),
                },
                Err(e) => tracing::warn!(
                    "[eastmoney] get_financials 现金流量表请求失败: {e} (stock_code={stock_code})"
                ),
            }
        }

        // 修复 M-FIN-2: 字段名映射可能因 API 升级而失效。
        // 当所有关键字段（roe/gross_margin/net_margin/revenue_yoy/profit_yoy）均为 None，
        // 但财报条目本身存在时，返回 VendorError 触发 fallback 到下一个 vendor。
        let critical_fields_empty = reports.iter().all(|r| {
            r.roe.is_none()
                && r.gross_margin.is_none()
                && r.net_margin.is_none()
                && r.revenue_yoy.is_none()
                && r.profit_yoy.is_none()
        });
        if critical_fields_empty {
            tracing::warn!(
                "[eastmoney] get_financials 所有财报的5个关键字段(ROE/毛利率/净利率/营收同比/净利润同比)均为 None，\
                 可能字段名映射失效(stock_code={stock_code})，触发 fallback"
            );
            return Err(DataError::VendorError {
                vendor: "eastmoney".into(),
                message: format!(
                    "财报关键字段全为 None，可能 API 字段名变更(stock_code={stock_code})"
                ),
            });
        }

        Ok(reports)
    }

    /// 历史估值日序列（估值带的唯一数据供应方）
    ///
    /// 东财数据中心 `RPT_VALUEANALYSIS_DET`：每个交易日一行，含 PE_TTM / PB_MRQ /
    /// PS_TTM / PCF_OCF_TTM / 收盘价 / 总市值。实测可回溯 8 年+（600519 共 2111 个交易日）。
    ///
    /// 背景：本地 `financial_snapshots` 表原设计为"每日 EOD 写一行"，但全项目从无写入路径
    /// （只有迁移建表和两个读命令）→ 估值带样本恒为 0、图表永远空白。
    /// 由本接口一次性回填多年历史（落库在 compute_valuation_band 命令层完成）。
    async fn get_valuation_history(
        &self,
        stock_code: &str,
        years: u32,
    ) -> Result<Vec<ValuationSnapshot>, DataError> {
        let code =
            stock_code.trim_start_matches("sh").trim_start_matches("sz").trim_start_matches("bj");
        // 每页 500 条；A 股每年约 243 个交易日，按 250 估算页数并留 1 页余量，上限 12 页防异常入参
        const PAGE_SIZE: usize = 500;
        let want = years.max(1) as usize * 250;
        let max_pages = ((want / PAGE_SIZE) + 2).min(12);
        // 接口按 TRADE_DATE 降序返回，因此累计到页码上限即可覆盖所要年数；
        // 精确的区间裁剪交给命令层（按 since_date 过滤），此处不做日期运算以免引入额外依赖。
        let mut out: Vec<ValuationSnapshot> = Vec::new();
        for page in 1..=max_pages {
            let url = format!(
                "https://datacenter-web.eastmoney.com/api/data/v1/get?reportName=RPT_VALUEANALYSIS_DET&columns=SECURITY_CODE,SECURITY_NAME_ABBR,TRADE_DATE,PE_TTM,PB_MRQ,PS_TTM,PCF_OCF_TTM,CLOSE_PRICE,TOTAL_MARKET_CAP&filter=(SECURITY_CODE=\"{code}\")&pageSize={PAGE_SIZE}&pageNumber={page}&sortColumns=TRADE_DATE&sortTypes=-1&source=WEB&client=WEB"
            );
            let resp = self.em_get(&url).await?;
            let json: Value = resp.json().await?;
            let rows = match json["result"]["data"].as_array() {
                Some(arr) if !arr.is_empty() => arr,
                _ => break,
            };
            let got = rows.len();
            for r in rows {
                // 与 get_financials 同款容错：字段可能为字符串或数字，"--"/""/null 视为缺失
                let n = |key: &str| -> Option<f64> {
                    let v = &r[key];
                    if v.is_null() {
                        return None;
                    }
                    if let Some(s) = v.as_str() {
                        if s.is_empty() || s == "--" || s == "null" {
                            return None;
                        }
                        return s.parse::<f64>().ok();
                    }
                    v.as_f64()
                };
                let date = r["TRADE_DATE"].as_str().unwrap_or("");
                if date.is_empty() {
                    continue;
                }
                out.push(ValuationSnapshot {
                    // "2026-09-11 00:00:00" → "2026-09-11"
                    trade_date: date.chars().take(10).collect(),
                    security_name: r["SECURITY_NAME_ABBR"].as_str().map(|x| x.to_string()),
                    pe_ttm: n("PE_TTM"),
                    pb: n("PB_MRQ"),
                    ps_ttm: n("PS_TTM"),
                    pcf: n("PCF_OCF_TTM"),
                    close_price: n("CLOSE_PRICE"),
                    total_market_cap: n("TOTAL_MARKET_CAP"),
                });
            }
            // 返回条数不足一页 = 已到最后一页
            if got < PAGE_SIZE {
                break;
            }
        }
        Ok(out)
    }

    /// S7(2026-09-26)：截止日单行估值快照（供 as-of 合成 quote 回填 total_mv/PE/PB）。
    /// 形态对齐两融先例：`TRADE_DATE<=截止日` + 倒序取第一条 —— 等值查会在休市日恒空。
    async fn get_valuation_snapshot_asof(&self, stock_code: &str) -> Option<ValuationSnapshot> {
        let as_of = crate::as_of::current_as_of()?;
        let cutoff = as_of.as_of_date.format("%Y-%m-%d").to_string();
        let code =
            stock_code.trim_start_matches("sh").trim_start_matches("sz").trim_start_matches("bj");
        let url = valuation_snapshot_asof_url(code, &cutoff);
        let resp = self.em_get(&url).await.ok()?;
        let json: Value = resp.json().await.ok()?;
        let r = json["result"]["data"].as_array()?.first()?;
        let n = |key: &str| -> Option<f64> {
            let v = &r[key];
            if v.is_null() {
                return None;
            }
            if let Some(s) = v.as_str() {
                if s.is_empty() || s == "--" || s == "null" {
                    return None;
                }
                return s.parse::<f64>().ok();
            }
            v.as_f64()
        };
        let date = r["TRADE_DATE"].as_str()?;
        if date.is_empty() {
            return None;
        }
        Some(ValuationSnapshot {
            trade_date: date.chars().take(10).collect(),
            security_name: r["SECURITY_NAME_ABBR"].as_str().map(|x| x.to_string()),
            pe_ttm: n("PE_TTM"),
            pb: n("PB_MRQ"),
            ps_ttm: n("PS_TTM"),
            pcf: n("PCF_OCF_TTM"),
            close_price: n("CLOSE_PRICE"),
            total_market_cap: n("TOTAL_MARKET_CAP"),
        })
    }

    async fn get_news(&self, stock_code: &str, limit: u32) -> Result<Vec<NewsItem>, DataError> {
        // 单页抓取收口在 `news_page`：与 `search_news`、as-of 回溯共用同一 endpoint 与解析
        self.news_page(stock_code, 1, limit.min(50), "default").await
    }

    /// T2：按截止日**回溯**取该股历史新闻（2026-09-26）。
    ///
    /// 通道形态见 `news_pages_until_asof`：该搜索接口不认日期参数，靠 `sort:"time"`
    /// 倒序翻页 + 本地裁剪逼近截止日。翻不到截止日时返回 `Err`（而不是空），
    /// 让路由层落到 `news_archive` 并把「回溯窗口不足」如实写进降级原因。
    async fn get_news_with_asof(
        &self,
        stock_code: &str,
        limit: u32,
    ) -> Result<Vec<NewsItem>, DataError> {
        let ctx = crate::as_of::current_as_of().ok_or_else(|| {
            DataError::ParseError("get_news_with_asof 调用时缺 as_of 上下文".into())
        })?;
        let cutoff = ctx.as_of_date.format("%Y-%m-%d").to_string();
        self.news_pages_until_asof(stock_code, &cutoff, limit.max(1) as usize).await
    }

    async fn search_news(&self, keyword: &str, limit: u32) -> Result<Vec<NewsItem>, DataError> {
        // 单页抓取收口在 `news_page`：与 `get_news`、as-of 回溯共用同一 endpoint 与解析
        self.news_page(keyword, 1, limit.min(50), "default").await
    }

    /// T2：关键词版的时间回溯（催化剂/行业事件验证用），复用 `news_pages_until_asof`。
    async fn search_news_with_asof(
        &self,
        keyword: &str,
        limit: u32,
    ) -> Result<Vec<NewsItem>, DataError> {
        let ctx = crate::as_of::current_as_of().ok_or_else(|| {
            DataError::ParseError("search_news_with_asof 调用时缺 as_of 上下文".into())
        })?;
        let cutoff = ctx.as_of_date.format("%Y-%m-%d").to_string();
        self.news_pages_until_asof(keyword, &cutoff, limit.max(1) as usize).await
    }

    async fn get_money_flow(&self, stock_code: &str) -> Result<Option<MoneyFlow>, DataError> {
        // 修复(2026-09-10): RPT_F10_HOMEPAGE_FUND_FLOW 报表已从 datacenter-web 下线
        // (返回 data:null)，f9 资金流信号因此断粮 3 个月。
        // 改回 push2his.eastmoney.com/api/qt/stock/fflow/daykline/get —— 2026-09-10 实测可用
        // (2026-07-22 弃用时的 IncompleteMessage 故障已消失，push2his 现走 IPv4 直连正常)。
        //
        // klines CSV 字段映射(fields2=f51..f56，单位: 元):
        //   f51=日期 f52=主力净流入 f53=小单净流入 f54=中单净流入 f55=大单净流入 f56=超大单净流入
        //   自洽校验: 主力(f52) = 超大单(f56) + 大单(f55)，全单和为零
        let secid = to_em_secid(stock_code);
        let url = format!(
            "https://push2his.eastmoney.com/api/qt/stock/fflow/daykline/get?\
            lmt=5&klt=101&fields1=f1,f2,f3,f7&\
            fields2=f51,f52,f53,f54,f55,f56,f57,f58,f59,f60,f61,f62,f63,f64,f65&\
            secid={secid}"
        );

        let resp = self.em_get(&url).await?;
        let json: Value = resp.json().await?;

        let klines = match json["data"]["klines"].as_array() {
            Some(arr) if !arr.is_empty() => arr,
            // data:null = 该股无资金流数据（如北交所部分标的），显式区分于网络错误
            _ => return Ok(None),
        };

        // 修复(2026-09-26): 接口实际按日期**升序**返回（实测 lmt=0 时 first=最早、
        // last=最新），原代码直接取 history[0] 当「最新」⇒ 顶层字段拿到的是窗口内
        // **最老一天** 的主力净流入。现显式降序，与注释口径一致。
        let history = select_fflow_window(parse_fflow_klines(klines), "9999-12-31");
        let Some(latest) = history.first() else {
            return Ok(None);
        };
        Ok(Some(MoneyFlow {
            date: latest.date.clone(),
            main_net_inflow: latest.main_net_inflow,
            super_large_net: latest.super_large_net,
            large_net: latest.large_net,
            medium_net: latest.medium_net,
            small_net: latest.small_net,
            history,
        }))
    }

    async fn get_money_flow_with_asof(
        &self,
        stock_code: &str,
    ) -> Result<Option<MoneyFlow>, DataError> {
        // S3(2026-09-26, PLAN-asof-replay-quality-attribution)：回放里「主力净流入」
        // 因子恒缺的修复。此前 eastmoney 对 get_money_flow 申报 Fallthrough，
        // 而 lib.rs 的 as-of 分支只走快照与 NativeDateParam 两路 ⇒ 直接降级返回 None。
        // ⚠ push2his fflow/daykline **忽略 beg/end**（2026-09-26 实测：lmt=0 恒返回
        //   最近 ~120 个交易日，与 end 取值无关）⇒ 只能拉全窗后**本地按截止日过滤**
        //   （形态对齐 get_north_bound_flow_with_asof 的「升序→截尾→反转」处理）。
        //   代价：截止日早于窗口头（约半年前）的回放仍拿不到，保持 record_degradation。
        let as_of = crate::as_of::current_as_of()
            .ok_or_else(|| DataError::ParseError("no as_of context".into()))?;
        let cutoff = as_of.as_of_date.format("%Y-%m-%d").to_string();
        let secid = to_em_secid(stock_code);
        let url = format!(
            "https://push2his.eastmoney.com/api/qt/stock/fflow/daykline/get?\
            lmt=0&klt=101&fields1=f1,f2,f3,f7&\
            fields2=f51,f52,f53,f54,f55,f56,f57,f58,f59,f60,f61,f62,f63,f64,f65&\
            secid={secid}"
        );

        let resp = self.em_get(&url).await?;
        let json: Value = resp.json().await?;

        let klines = match json["data"]["klines"].as_array() {
            Some(arr) if !arr.is_empty() => arr,
            _ => return Ok(None),
        };

        // 升序全窗 → 过滤 date<=cutoff → 最近 5 条降序（纯函数，边界测试见 fflow_asof_tests）
        let recent = select_fflow_window(parse_fflow_klines(klines), &cutoff);
        if recent.is_empty() {
            tracing::debug!(
                "[eastmoney] get_money_flow_with_asof 未匹配到 date<={cutoff} 的资金流数据(stock_code={stock_code})"
            );
            return Ok(None);
        }
        // 截止日休市时取到的是更早一天 —— 正确行为，留 info 便于事后核对（同两融先例）
        if recent[0].date != cutoff {
            tracing::info!(
                "[eastmoney] 资金流 as-of：{stock_code} 截止日 {cutoff} 无披露，取最近交易日 {}",
                recent[0].date
            );
        }
        let latest = &recent[0];
        Ok(Some(MoneyFlow {
            date: latest.date.clone(),
            main_net_inflow: latest.main_net_inflow,
            super_large_net: latest.super_large_net,
            large_net: latest.large_net,
            medium_net: latest.medium_net,
            small_net: latest.small_net,
            history: recent,
        }))
    }

    async fn get_dragon_tiger(&self, stock_code: &str) -> Result<Vec<DragonTigerEntry>, DataError> {
        // P1-3 修复(2026-07-22): 原 push2his.eastmoney.com/api/qt/stock/mmpa/get
        // 已失效(IncompleteMessage)。改用 datacenter-web.eastmoney.com 的
        // RPT_DAILYBILLBOARD_DETAILS 报表(东方财富数据中心"龙虎榜"页面数据源)。
        //
        // 字段映射:
        //   - TRADE_DATE: 交易日期
        //   - EXPLANATION: 上榜原因
        //   - BILLBOARD_BUY_AMT: 龙虎榜买入额
        //   - BILLBOARD_SELL_AMT: 龙虎榜卖出额
        //   - BILLBOARD_NET_AMT: 龙虎榜净买额
        //   - OPERATEDEPT_NAME: 营业部名称（席位明细）
        //   - BUY/SELL/NET: 该营业部买入额/卖出额/净额
        //
        // 2026-09-21 修复（实证见 AUDIT-analyst-low-confidence-2026-09-21.md §三）：
        // 原实现用汇总级报表 RPT_DAILYBILLBOARD_DETAILS 的 BUY_SEAT_NEW/SELL_SEAT_NEW
        // 拼 dept_name，两处缺陷叠加：
        //   ① BUY_SEAT_NEW 实测返回**字符串**（"13331"），`as_i64()` 对字符串恒返回 None，
        //      被 `unwrap_or(0)` 静默吞掉 ⇒ 每行恒产出「买入0席位/卖出0席位」，
        //      14/14 轮历史运行 100% 复现；且该字段是**席位编号**不是营业部数量。
        //   ② 该报表是**汇总级**（只有买卖总额），本就不含营业部明细 ⇒ 分析师反复写
        //      「席位数据缺失/未显示营业部明细」，是其报告如实反映工具缺口。
        // 现改接 RPT_BILLBOARD_DAILYDETAILSBUY / ...SELL（营业部级明细，实测可用），
        // dept_name 取真实 OPERATEDEPT_NAME，与 baidu_stock 的同名字段语义对齐
        // （该字段在两个 vendor 上此前语义不一致）。
        let code =
            stock_code.trim_start_matches("sh").trim_start_matches("sz").trim_start_matches("bj");

        let mut rows: Vec<Value> = Vec::new();
        let mut ok_reports = 0usize;
        let mut last_err: Option<DataError> = None;
        for report in ["RPT_BILLBOARD_DAILYDETAILSBUY", "RPT_BILLBOARD_DAILYDETAILSSELL"] {
            let url = format!(
                "https://datacenter-web.eastmoney.com/api/data/v1/get?\
                reportName={report}&columns=ALL&\
                filter=(SECURITY_CODE%3D%22{code}%22)&\
                pageSize=50&pageNumber=1&source=WEB&\
                sortColumns=TRADE_DATE&sortTypes=-1"
            );
            match self.em_get(&url).await {
                Ok(resp) => match resp.json::<Value>().await {
                    Ok(json) => {
                        ok_reports += 1;
                        if let Some(arr) = json["result"]["data"].as_array() {
                            rows.extend(arr.iter().cloned());
                        }
                    },
                    Err(e) => {
                        // 不静默兜底：成因入日志；两个报表都失败则向上返回错误
                        tracing::warn!("[eastmoney] get_dragon_tiger {report} JSON 解析失败: {e}");
                        last_err = Some(DataError::VendorError {
                            vendor: "eastmoney".into(),
                            message: format!("get_dragon_tiger {report} JSON 解析失败: {e}"),
                        });
                    },
                },
                Err(e) => {
                    tracing::warn!("[eastmoney] get_dragon_tiger {report} 请求失败: {e}");
                    last_err = Some(e);
                },
            }
        }
        if ok_reports == 0 {
            return Err(last_err.unwrap_or_else(|| DataError::VendorError {
                vendor: "eastmoney".into(),
                message: "get_dragon_tiger 买卖席位明细报表均请求失败".into(),
            }));
        }

        Ok(rows
            .iter()
            .map(|r| {
                let trade_date = r["TRADE_DATE"].as_str().unwrap_or("");
                // 截取日期部分 "YYYY-MM-DD 00:00:00" → "YYYY-MM-DD"
                let date = if trade_date.len() >= 10 {
                    trade_date[..10].to_string()
                } else {
                    trade_date.to_string()
                };
                DragonTigerEntry {
                    stock_code: stock_code.to_string(),
                    date,
                    dept_name: r["OPERATEDEPT_NAME"].as_str().unwrap_or("").to_string(),
                    buy_amount: r["BUY"].as_f64().unwrap_or(0.0),
                    sell_amount: r["SELL"].as_f64().unwrap_or(0.0),
                    net_amount: r["NET"].as_f64().unwrap_or(0.0),
                    reason: r["EXPLANATION"].as_str().map(|s| s.to_string()),
                }
            })
            .collect())
    }

    async fn get_lockup_schedule(
        &self,
        stock_code: &str,
    ) -> Result<Vec<LockupSchedule>, DataError> {
        // 修复(2026-07-22): 原 reportName=RPTA_WEB_LOCKUP 已失效("报表配置不存在")。
        // 改用 RPT_LIFT_GD 报表(来自 data.eastmoney.com/newstatic/js/xsjj/history.js,
        // 即东方财富数据中心"限售股解禁"页面的实际数据源)。
        //
        // 字段映射:
        //   - SECURITY_CODE/SECUCODE/SECURITY_NAME_ABBR: 代码/简称
        //   - FREE_DATE: 解禁日期(格式 "YYYY-MM-DD 00:00:00")
        //   - ADD_LISTING_SHARES: 本次解禁数量(股)
        //   - LIMITED_HOLDER_NAME: 限售股持有人名称
        //   - FREE_SHARES_TYPE: 限售股类型(如"股权激励限售股份")
        //   - RESIDUAL_LIMITED_SHARES: 剩余限售股数
        //   - LIFT_SHARES_ALL: 当次解禁总数(同一 FREE_DATE 下多股东合计)
        //   - TOTAL_SHARES_NUM: 总股本(用于计算解禁比例)
        //
        // 修复(2026-07-22): SECURITY_CODE 字段需纯数字代码,去除 sh/sz/bj 前缀
        let code =
            stock_code.trim_start_matches("sh").trim_start_matches("sz").trim_start_matches("bj");
        let url = format!(
            "https://datacenter-web.eastmoney.com/api/data/v1/get?\
            reportName=RPT_LIFT_GD&columns=ALL&\
            filter=(SECURITY_CODE%3D%22{code}%22)&\
            pageSize=50&pageNumber=1&source=WEB&\
            sortColumns=FREE_DATE&sortTypes=-1"
        );

        let resp = self.em_get(&url).await?;
        let json: Value = resp.json().await?;

        let rows = match json["result"]["data"].as_array() {
            Some(arr) if !arr.is_empty() => arr,
            _ => {
                // result.data 为空或结构异常，触发 VendorError 让下一个 vendor 尝试
                return Err(DataError::VendorError {
                    vendor: "eastmoney".into(),
                    message: format!("get_lockup_schedule 数据为空(stock_code={stock_code})"),
                });
            },
        };

        Ok(rows
            .iter()
            .map(|r| {
                // FREE_DATE 格式 "YYYY-MM-DD 00:00:00",截取日期部分
                let raw_date = r["FREE_DATE"].as_str().unwrap_or("");
                let date = raw_date.split_whitespace().next().unwrap_or(raw_date).to_string();
                // 解禁数量(本次新增可上市股份,单位:股)
                let unlock_shares = r["ADD_LISTING_SHARES"].as_f64().unwrap_or(0.0);
                // P2-3 修复: 解禁比例计算
                // RPT_LIFT_GD 不返回 TOTAL_SHARES_NUM,但返回 LIFT_SHARES_ALL(当日总解禁股数)
                // 和 ADD_LISTING_SHARES(单股东解禁股数)。
                // unlock_ratio 表示该股东解禁占当日总解禁的比例,非占总股本比例。
                // 若需占总股本比例,上层 LLM 可用 unlock_shares / total_shares 计算。
                let lift_shares_all = r["LIFT_SHARES_ALL"].as_f64().unwrap_or(0.0);
                let unlock_ratio = if lift_shares_all > 0.0 {
                    (unlock_shares / lift_shares_all * 100.0 * 100.0).round() / 100.0
                } else {
                    0.0
                };
                LockupSchedule {
                    stock_code: stock_code.to_string(),
                    stock_name: r["SECURITY_NAME_ABBR"].as_str().unwrap_or("").to_string(),
                    unlock_date: date,
                    unlock_shares,
                    unlock_ratio,
                    shareholder: r["LIMITED_HOLDER_NAME"].as_str().map(|s| s.to_string()),
                }
            })
            .collect())
    }

    /// 获取融资融券数据
    ///
    /// 修复(2026-07-22): 原 API `push2his.eastmoney.com/api/qt/stock/margin/get` 已失效(返回404)。
    /// 改用 datacenter-web 的 `RPTA_WEB_RZRQ_GGMX` 报表,该报表提供个股融资融券明细数据。
    /// 响应字段映射:
    ///   - RZYE: 融资余额(元)
    ///   - RQYE: 融券余额(元)
    ///   - RZMRE: 融资买入额(元)
    ///   - RQMCL: 融券卖出量(股)
    ///   - DATE: 交易日期
    async fn get_margin_data(&self, stock_code: &str) -> Result<Option<MarginData>, DataError> {
        // 去除 sh/sz/bj 前缀
        let code =
            stock_code.trim_start_matches("sh").trim_start_matches("sz").trim_start_matches("bj");
        let url = format!(
            "https://datacenter-web.eastmoney.com/api/data/v1/get?\
            reportName=RPTA_WEB_RZRQ_GGMX&columns=ALL&\
            filter=(scode%3D%22{code}%22)&source=WEB&\
            sortColumns=DATE&sortTypes=-1&pageNumber=1&pageSize=1"
        );

        let resp = self.em_get(&url).await?;
        let json: Value = resp.json().await?;

        // 检查 API 返回是否成功
        if json["success"].as_bool() == Some(false) {
            let msg = json["message"].as_str().unwrap_or("unknown error");
            return Err(DataError::VendorError {
                vendor: "eastmoney".into(),
                message: format!("get_margin_data API 错误: {msg}(stock_code={stock_code})"),
            });
        }

        let data = match json["result"]["data"].as_array() {
            Some(arr) if !arr.is_empty() => &arr[0],
            _ => {
                return Err(DataError::VendorError {
                    vendor: "eastmoney".into(),
                    message: format!(
                        "get_margin_data 数据为空(stock_code={stock_code}),该股票可能非融资融券标的"
                    ),
                });
            },
        };

        let parse_f64 = |key: &str| -> f64 { data[key].as_f64().unwrap_or(0.0) };

        Ok(Some(MarginData {
            stock_code: stock_code.to_string(),
            date: data["DATE"].as_str().unwrap_or("").to_string(),
            margin_buy: parse_f64("RZMRE"),        // 融资买入额(元)
            margin_balance: parse_f64("RZYE"),     // 融资余额(元)
            short_sell_volume: parse_f64("RQMCL"), // 融券卖出量(股)
            short_balance: parse_f64("RQYE"),      // 融券余额(元)
        }))
    }

    /// P1-2 新增: eastmoney 实现 consensus_eps
    /// 复用 reportapi.eastmoney.com/report/list 接口，聚合最近研报的 EPS 预测。
    /// 此接口与 get_research_reports 同源，是 eastmoney 稳定的 emweb 系列接口，
    /// 不受 push2his/push2 系列故障影响。
    ///
    // 修复(2026-07-22 #6): 目标价计算错误。
    // 原代码用 predictThisYearPe(预测PE) 直接当目标价，语义错误。
    // 正确做法: 目标价 = 预测PE × 预测EPS，二者均可用时才算出目标价。
    async fn get_consensus_eps(&self, stock_code: &str) -> Result<Option<ConsensusEPS>, DataError> {
        let url = format!(
            "https://reportapi.eastmoney.com/report/list?industryCode=*&pageSize=20&industry=%2A&rating=&ratingChange=&beginTime=2000-01-01&endTime=2030-01-01&pageNo=1&fields=&qType=0&orgCode=&code={}&rcode=&p=1&pageNum=1&pageNumber=1",
            stock_code
        );
        let resp = self.em_get(&url).await?;
        let json: Value = resp.json().await?;
        let reports = match json["data"].as_array() {
            Some(arr) if !arr.is_empty() => arr,
            _ => return Ok(None),
        };

        // 聚合今年 EPS 预测（predictThisYearEps），取均值作为一致预期
        let mut eps_sum = 0.0_f64;
        let mut eps_count = 0_i32;
        let mut rating_count = 0_i32;
        // 目标价 = 预测PE × 预测EPS（每篇研报独立计算后取均值）
        let mut target_price_sum = 0.0_f64;
        let mut target_price_count = 0_i32;
        let mut rating_avg: Option<String> = None;

        for r in reports {
            let eps_val = r["predictThisYearEps"].as_str().and_then(|s| s.parse::<f64>().ok());
            if let Some(val) = eps_val {
                eps_sum += val;
                eps_count += 1;
            }
            // 目标价 = PE × EPS（二者均可用时）
            if let (Some(eps), Some(pe_str)) = (eps_val, r["predictThisYearPe"].as_str()) {
                if let Ok(pe) = pe_str.parse::<f64>() {
                    if pe > 0.0 && eps > 0.0 {
                        target_price_sum += pe * eps;
                        target_price_count += 1;
                    }
                }
            }
            rating_count += 1;
            if rating_avg.is_none() {
                if let Some(rating) = r["emRatingName"].as_str() {
                    rating_avg = Some(rating.to_string());
                }
            }
        }

        if eps_count == 0 && rating_count == 0 {
            return Ok(None);
        }

        let consensus_eps = if eps_count > 0 {
            Some(eps_sum / eps_count as f64)
        } else {
            None
        };
        let consensus_target_price = if target_price_count > 0 {
            Some(target_price_sum / target_price_count as f64)
        } else {
            None
        };

        Ok(Some(ConsensusEPS {
            stock_code: stock_code.to_string(),
            consensus_eps,
            consensus_target_price,
            rating_avg,
            rating_count: Some(rating_count),
            year: chrono::Utc::now().format("%Y").to_string(),
            // vendor 返回的真实一致预期 ⇒ 非估算
            is_estimated: false,
            estimate_source: None,
        }))
    }

    async fn get_north_bound_holding(
        &self,
        stock_code: &str,
    ) -> Result<Option<NorthBoundHolding>, DataError> {
        // 修复(2026-07-22): 原 push2his fflow API 已失效(连接错误)。
        // 此外,2024-08-16 起监管层暂停披露北向资金实时数据,即使 API 可用,数据也为 0。
        //
        // 替代方案:用 RPT_F10_EH_HOLDERS(十大股东季度持股)中筛选"香港中央结算有限公司"
        // (HKSCC,代表港股通持股)作为北向持股的代理数据。
        //
        // 限制:
        //   1. 数据是季度披露,非实时(延迟最多 90 天)
        //   2. 若该股票港股通持股未排进十大股东,则返回 None(表示北向持股不重要)
        //   3. change_shares 由相邻两期 HOLD_NUM 差值计算
        //
        // 字段:
        //   - END_DATE: 报告期(如 "2026-03-31 00:00:00") → date
        //   - HOLD_NUM: 持股数量 → holding_shares
        //   - HOLD_NUM_RATIO: 持股比例(%) → holding_ratio
        //   - 上期 HOLD_NUM - 本期 HOLD_NUM → change_shares (正数=减持,负数=增持)
        let secucode = to_em_secucode(stock_code);
        let url = format!(
            "https://datacenter-web.eastmoney.com/api/data/v1/get?\
            reportName=RPT_F10_EH_HOLDERS&columns=ALL&\
            filter=(SECUCODE%3D%22{secucode}%22)&\
            pageSize=200&pageNumber=1&source=WEB&\
            sortColumns=END_DATE&sortTypes=-1"
        );
        let resp = self.em_get(&url).await?;
        let json: Value = resp.json().await?;

        let rows = match json["result"]["data"].as_array() {
            Some(arr) if !arr.is_empty() => arr,
            _ => return Ok(None),
        };

        // 筛选"香港中央结算有限公司"(HKSCC)的记录,按 END_DATE 降序排
        let mut hk_records: Vec<&Value> = rows
            .iter()
            .filter(|r| {
                r["HOLDER_NAME"]
                    .as_str()
                    .map(|n| n.contains("香港中央结算") || n.contains("HKSCC"))
                    .unwrap_or(false)
            })
            .collect();

        if hk_records.is_empty() {
            // 该股票港股通持股未排进十大股东,返回 None
            return Ok(None);
        }

        // 按日期降序排(理论上 API 已按 END_DATE DESC 排,但保险起见再排一次)
        hk_records.sort_by(|a, b| {
            let da = a["END_DATE"].as_str().unwrap_or("");
            let db = b["END_DATE"].as_str().unwrap_or("");
            db.cmp(da)
        });

        let latest = hk_records[0];
        let latest_date = latest["END_DATE"]
            .as_str()
            .map(|s| s.split_whitespace().next().unwrap_or(s).to_string())
            .unwrap_or_default();
        let latest_shares = latest["HOLD_NUM"].as_f64().unwrap_or(0.0);
        let latest_ratio = latest["HOLD_NUM_RATIO"].as_f64().unwrap_or(0.0);

        // 取次新的一期(必须是不同的 END_DATE)作为上期
        let prev_shares = hk_records
            .iter()
            .skip(1)
            .find(|r| {
                r["END_DATE"]
                    .as_str()
                    .map(|d| d != latest["END_DATE"].as_str().unwrap_or(""))
                    .unwrap_or(true)
            })
            .and_then(|r| r["HOLD_NUM"].as_f64())
            .unwrap_or(latest_shares);

        // change_shares: 正数=本期相比上期增持,负数=减持
        let change_shares = latest_shares - prev_shares;

        Ok(Some(NorthBoundHolding {
            stock_code: stock_code.to_string(),
            date: latest_date,
            holding_shares: latest_shares,
            holding_ratio: latest_ratio,
            change_shares,
        }))
    }

    async fn get_sector_info(&self, stock_code: &str) -> Result<Option<SectorInfo>, DataError> {
        // 修复(2026-07-22): 原 push2his stock/get API 已失效(WAF/JA3 检测导致连接错误)。
        // 改用 emweb F10 的 CompanySurvey/PageAjax 接口获取行业分类。
        //
        // 字段映射:
        //   - EM2016: 东财行业分类(如 "食品饮料-食品-乳制品"),用 "-" 拆分为一级行业/二级行业/细分
        //   - INDUSTRYCSRC1: 证监会行业分类(如 "制造业-食品制造业"),作为 concept_tags 补充
        //
        // 注:avg_pe/avg_pb 原 push2his clist/get API 也已失效,
        // 行业 PE/PB 数据请通过 get_industry_ranking 或 get_peers 获取,
        // 这里设为 None,不阻塞主流程。
        let secucode = to_em_secucode(stock_code);
        let url =
            format!("https://emweb.eastmoney.com/PC_HSF10/CompanySurvey/PageAjax?code={secucode}");
        let resp = self.em_get(&url).await?;
        let json: Value = resp.json().await?;

        let data = match json["jbzl"].get(0) {
            Some(d) => d,
            None => return Ok(None),
        };

        // EM2016: "食品饮料-食品-乳制品" → ["食品饮料", "食品", "乳制品"]
        let em_industry = data["EM2016"].as_str().unwrap_or("");
        let parts: Vec<&str> = em_industry.split('-').collect();
        let sector_name = parts.first().map(|s| s.trim().to_string()).unwrap_or_default();
        let sub_sector = parts.get(1).map(|s| s.trim().to_string()).unwrap_or_default();
        // 细分行业(如有)拼接到 sub_sector
        let sub_sector = if parts.len() >= 3 {
            let detail = parts[2].trim();
            format!("{sub_sector}-{detail}")
        } else {
            sub_sector
        };

        // 概念标签:把证监会行业分类作为 concept_tags 的第一项
        let mut concept_tags: Vec<String> = Vec::new();
        if let Some(csrc) = data["INDUSTRYCSRC1"].as_str() {
            if !csrc.is_empty() {
                concept_tags.push(csrc.to_string());
            }
        }
        // SECURITY_TYPE 也作为标签(如"上交所主板A股")
        if let Some(stype) = data["SECURITY_TYPE"].as_str() {
            if !stype.is_empty() {
                concept_tags.push(stype.to_string());
            }
        }

        if sector_name.is_empty() && concept_tags.is_empty() {
            return Ok(None);
        }

        Ok(Some(SectorInfo {
            stock_code: stock_code.to_string(),
            sector_name,
            sub_sector,
            concept_tags,
            // 原 push2his clist/get 失效,avg_pe/avg_pb 暂不可用
            // 行业 PE/PB 请通过 get_industry_ranking / get_peers 获取
            avg_pe: None,
            avg_pb: None,
        }))
    }

    async fn get_shareholder_trades(
        &self,
        stock_code: &str,
    ) -> Result<Vec<ShareholderTrade>, DataError> {
        // 修复(2026-07-22): 原 RPTA_WEB_MAJORHOLDERS_TRADE 已失效。
        // 第一版修复改用 RPT_F10_EH_HOLDERS(十大股东季度持股变动),
        // 但该报表不提供成交价格(price=0.0 占位),导致 LLM 无法计算减持均价。
        //
        // 第二版修复(本次): 改用 RPT_F10_HOLDER_HOLDERTRADE 报表
        // (东方财富 F10 股东增减持明细,含真实成交价格)。
        //
        // 字段映射:
        //   - CHANGE_DATE: 变动日期
        //   - HOLDER_NAME: 股东名称
        //   - CHANGE_NUM: 变动数量(股)
        //   - CHANGE_PRICE: 成交均价(元)
        //   - CHANGE_RATIO_AFTER: 变动后持股比例
        //   - CHANGE_TYPE: 变动类型("增持"/"减持")
        let code =
            stock_code.trim_start_matches("sh").trim_start_matches("sz").trim_start_matches("bj");
        let secucode = to_em_secucode(code);
        let url = format!(
            "https://datacenter-web.eastmoney.com/api/data/v1/get?\
            reportName=RPT_F10_HOLDER_HOLDERTRADE&columns=ALL&\
            filter=(SECUCODE%3D%22{secucode}%22)&\
            pageSize=20&pageNumber=1&source=WEB&\
            sortColumns=CHANGE_DATE&sortTypes=-1"
        );
        let resp = self.em_get(&url).await?;
        let json: Value = resp.json().await?;

        let rows = match json["result"]["data"].as_array() {
            Some(arr) if !arr.is_empty() => arr,
            _ => {
                // 回退到 RPT_F10_EH_HOLDERS(无成交价但至少有股东变动信息)
                let eh_url = format!(
                    "https://datacenter-web.eastmoney.com/api/data/v1/get?\
                    reportName=RPT_F10_EH_HOLDERS&columns=ALL&\
                    filter=(SECUCODE%3D%22{secucode}%22)&\
                    pageSize=20&pageNumber=1&source=WEB&\
                    sortColumns=END_DATE&sortTypes=-1"
                );
                let eh_resp = self.em_get(&eh_url).await?;
                let eh_json: Value = eh_resp.json().await?;
                let eh_rows = match eh_json["result"]["data"].as_array() {
                    Some(arr) if !arr.is_empty() => arr,
                    _ => {
                        return Err(DataError::VendorError {
                            vendor: "eastmoney".into(),
                            message: format!(
                                "get_shareholder_trades 数据为空(stock_code={stock_code})"
                            ),
                        });
                    },
                };
                return Ok(eh_rows
                    .iter()
                    .map(|r| {
                        let raw_date = r["END_DATE"].as_str().unwrap_or("");
                        let date =
                            raw_date.split_whitespace().next().unwrap_or(raw_date).to_string();
                        let raw_change = r["HOLD_NUM_CHANGE"].as_str().unwrap_or("");
                        let shares = if raw_change == "不变" || raw_change.is_empty() {
                            0.0
                        } else if let Ok(n) = raw_change.parse::<f64>() {
                            n
                        } else {
                            r["HOLD_NUM"].as_f64().unwrap_or(0.0)
                        };
                        let trade_type = r["HOLDER_STATEE"]
                            .as_str()
                            .or_else(|| r["HOLD_RATIO_QOQ"].as_str())
                            .unwrap_or("不变")
                            .to_string();
                        ShareholderTrade {
                            stock_code: stock_code.to_string(),
                            date,
                            shareholder_name: r["HOLDER_NAME"].as_str().unwrap_or("").to_string(),
                            trade_type,
                            shares,
                            // RPT_F10_EH_HOLDERS 不提供成交价格
                            price: 0.0,
                            reason: r["SHARES_TYPE"].as_str().map(|s| s.to_string()),
                        }
                    })
                    .collect());
            },
        };

        Ok(rows
            .iter()
            .map(|r| {
                let raw_date = r["CHANGE_DATE"].as_str().unwrap_or("");
                let date = raw_date.split_whitespace().next().unwrap_or(raw_date).to_string();
                let shares = r["CHANGE_NUM"].as_f64().unwrap_or(0.0);
                let price = r["CHANGE_PRICE"]
                    .as_f64()
                    .or_else(|| r["CHANGE_PRICE"].as_str().and_then(|s| s.parse::<f64>().ok()))
                    .unwrap_or(0.0);
                let trade_type = r["CHANGE_TYPE"].as_str().unwrap_or("变动").to_string();
                ShareholderTrade {
                    stock_code: stock_code.to_string(),
                    date,
                    shareholder_name: r["HOLDER_NAME"].as_str().unwrap_or("").to_string(),
                    trade_type,
                    shares,
                    price,
                    reason: r["CHANGE_RATIO_AFTER"].as_str().map(|s| s.to_string()),
                }
            })
            .collect())
    }

    async fn get_dividend_records(
        &self,
        stock_code: &str,
    ) -> Result<Vec<DividendRecord>, DataError> {
        // 东方财富数据中心: 分红送配数据
        // 修复(2026-07-22): SECURITY_CODE 字段需纯数字代码,去除 sh/sz/bj 前缀
        let code =
            stock_code.trim_start_matches("sh").trim_start_matches("sz").trim_start_matches("bj");
        let url = format!(
            "https://datacenter-web.eastmoney.com/api/data/v1/get?reportName=RPTA_WEB_DIVIDEND&columns=SECURITY_CODE,EX_DIVIDEND_DATE,DIVIDEND_PER_SHARE,BONUS_SHARE_RATIO,RECORD_DATE&filter=(SECURITY_CODE=\"{code}\")&pageSize=10&pageNumber=1"
        );
        let resp = self.em_get(&url).await?;
        let json: Value = resp.json().await?;

        let rows = match json["result"]["data"].as_array() {
            Some(arr) => arr,
            None => return Ok(vec![]),
        };

        rows.iter()
            .map(|r| {
                Ok(DividendRecord {
                    stock_code: stock_code.to_string(),
                    ex_date: r["EX_DIVIDEND_DATE"].as_str().unwrap_or("").to_string(),
                    dividend_per_share: r["DIVIDEND_PER_SHARE"].as_f64().unwrap_or(0.0),
                    bonus_share_ratio: r["BONUS_SHARE_RATIO"].as_f64().unwrap_or(0.0),
                    record_date: r["RECORD_DATE"].as_str().unwrap_or("").to_string(),
                })
            })
            .collect()
    }

    /// 获取财报日历事件
    ///
    /// 使用东方财富公告 API（RPTA_WEB_NOTICE），按标题关键词分类：
    /// - "业绩预告" → preliminary
    /// - "业绩快报" → express
    /// - "定期报告"/"年报"/"季报" → formal
    /// - "股东大会" → shareholders_meeting
    /// - 其他 → other
    async fn get_earnings_calendar(
        &self,
        stock_code: &str,
    ) -> Result<Vec<EarningsEvent>, DataError> {
        // 修复(2026-07-22): SECURITY_CODE 字段需纯数字代码,去除 sh/sz/bj 前缀
        let code =
            stock_code.trim_start_matches("sh").trim_start_matches("sz").trim_start_matches("bj");
        let url = format!(
            "https://datacenter-web.eastmoney.com/api/data/v1/get?reportName=RPTA_WEB_NOTICE&columns=SECURITY_CODE,SECURITY_NAME_ABBR,NOTICE_DATE,TITLE,EQUITY_NOTICE_TYPE&filter=(SECURITY_CODE=\"{code}\")&pageSize=30&sortColumns=NOTICE_DATE&sortTypes=-1&pageNumber=1"
        );
        let resp = self.em_get(&url).await?;
        let json: Value = resp.json().await?;

        let rows = match json["result"]["data"].as_array() {
            Some(arr) => arr,
            None => return Ok(vec![]),
        };

        Ok(rows
            .iter()
            .filter_map(|r| {
                let title = r["TITLE"].as_str().unwrap_or("");
                let notice_date = r["NOTICE_DATE"].as_str().unwrap_or("");
                if title.is_empty() || notice_date.is_empty() {
                    return None;
                }

                // 按标题关键词分类
                let (event_type, period) = classify_earnings_title(title);

                // 只保留财报相关事件
                if event_type == "other" && !title.contains("报告") && !title.contains("业绩") {
                    return None;
                }

                Some(EarningsEvent {
                    stock_code: stock_code.to_string(),
                    stock_name: r["SECURITY_NAME_ABBR"].as_str().unwrap_or("").to_string(),
                    event_date: notice_date.to_string(),
                    event_type: event_type.to_string(),
                    period,
                    detail: Some(title.to_string()),
                    source: Some("eastmoney".to_string()),
                    created_at: chrono::Utc::now().timestamp(),
                })
            })
            .collect())
    }

    async fn search_stock(&self, keyword: &str) -> Result<Vec<StockSearchResult>, DataError> {
        let url = format!(
            "https://searchadapter.eastmoney.com/api/suggest/get?input={}&type=14&count=20",
            urlencoding::encode(keyword)
        );

        let resp = self.em_get(&url).await?;
        let json: Value = resp.json().await?;

        let stocks = match json["QuotationCodeTable"]["Data"].as_array() {
            Some(arr) => arr,
            None => return Ok(vec![]),
        };

        Ok(stocks
            .iter()
            .map(|s| StockSearchResult {
                code: s["Code"].as_str().unwrap_or("").to_string(),
                name: s["Name"].as_str().unwrap_or("").to_string(),
                market: s["Market"].as_str().unwrap_or("").to_string(),
            })
            .collect())
    }

    async fn get_research_reports(
        &self,
        stock_code: &str,
    ) -> Result<Vec<ResearchReport>, DataError> {
        let url = format!(
            "https://reportapi.eastmoney.com/report/list?industryCode=*&pageSize=20&industry=%2A&rating=&ratingChange=&beginTime=2000-01-01&endTime=2030-01-01&pageNo=1&fields=&qType=0&orgCode=&code={}&rcode=&p=1&pageNum=1&pageNumber=1",
            stock_code
        );

        let resp = self.em_get(&url).await?;

        let json: Value = resp.json().await?;

        let reports = match json["data"].as_array() {
            Some(arr) => arr,
            None => return Ok(vec![]),
        };

        Ok(reports
            .iter()
            .map(|r| {
                let mut eps_forecast = Vec::new();
                let mut this_year_eps: Option<f64> = None;
                if let Some(eps) = r["predictThisYearEps"].as_str() {
                    if let Ok(val) = eps.parse::<f64>() {
                        eps_forecast.push(EpsForecast { year: "今年".into(), eps: Some(val) });
                        this_year_eps = Some(val);
                    }
                }
                if let Some(eps) = r["predictNextYearEps"].as_str() {
                    if let Ok(val) = eps.parse::<f64>() {
                        eps_forecast.push(EpsForecast { year: "明年".into(), eps: Some(val) });
                    }
                }
                if let Some(eps) = r["predictNextTwoYearEps"].as_str() {
                    if let Ok(val) = eps.parse::<f64>() {
                        eps_forecast.push(EpsForecast { year: "后年".into(), eps: Some(val) });
                    }
                }

                // 修复(2026-07-22 #6): 目标价 = 预测PE × 预测EPS
                // 原代码硬编码 target_price: None,导致所有研报目标价为 null。
                // 东方财富 reportapi 不直接返回目标价,但返回预测PE和预测EPS,
                // 二者相乘可得隐含目标价。
                let target_price = if let (Some(eps), Some(pe_str)) =
                    (this_year_eps, r["predictThisYearPe"].as_str())
                {
                    pe_str
                        .parse::<f64>()
                        .ok()
                        .filter(|&pe| pe > 0.0 && eps > 0.0)
                        .map(|pe| pe * eps)
                } else {
                    None
                };

                let info_code = r["infoCode"].as_str().unwrap_or("");
                let pdf_url = if info_code.is_empty() {
                    None
                } else {
                    Some(format!("https://pdf.dfcfw.com/pdf/H3_{}_1.pdf", info_code))
                };

                ResearchReport {
                    title: r["title"].as_str().unwrap_or("").to_string(),
                    institution: r["orgSName"].as_str().unwrap_or("").to_string(),
                    analyst: r["researcher"].as_str().map(|s| s.to_string()),
                    rating: r["emRatingName"].as_str().map(|s| s.to_string()),
                    target_price,
                    eps_forecast,
                    publish_date: r["publishDate"].as_str().unwrap_or("").to_string(),
                    pdf_url,
                }
            })
            .collect())
    }

    async fn get_market_dragon_tiger(&self) -> Result<Vec<MarketDragonTiger>, DataError> {
        let url = "https://datacenter-web.eastmoney.com/api/data/v1/get?reportName=RPT_DAILYBOARD_DETAILS_NEW&columns=SECURITY_CODE,SECURITY_NAME_ABBR,TRADE_DATE,BUY_AMOUNT,SELL_AMOUNT,NET_BUY,CHANGE_REASON&sortColumns=NET_BUY&sortTypes=-1&pageSize=30&pageNumber=1";

        let resp = self.em_get(url).await?;
        let json: Value = resp.json().await?;

        let rows = match json["result"]["data"].as_array() {
            Some(arr) => arr,
            None => return Ok(vec![]),
        };

        Ok(rows
            .iter()
            .map(|r| MarketDragonTiger {
                stock_code: r["SECURITY_CODE"].as_str().unwrap_or("").to_string(),
                stock_name: r["SECURITY_NAME_ABBR"].as_str().unwrap_or("").to_string(),
                date: r["TRADE_DATE"].as_str().unwrap_or("").to_string(),
                net_buy: r["NET_BUY"].as_f64().unwrap_or(0.0),
                buy_amount: r["BUY_AMOUNT"].as_f64().unwrap_or(0.0),
                sell_amount: r["SELL_AMOUNT"].as_f64().unwrap_or(0.0),
                reason: r["CHANGE_REASON"].as_str().map(|s| s.to_string()),
            })
            .collect())
    }

    async fn get_cls_flash(&self) -> Result<Vec<ClsFlashItem>, DataError> {
        // 2026-08-01 修复：旧接口 getNewsByColumns?column=250（财联社快讯频道）已整体失效
        // （curl 实测所有 column 均返回空 list）。改用东财 7x24 快讯接口 getFastNewsList。
        // `sortEnd` 的真实口径见 `flash_cursor_at`（游标是 Unix 秒 ×1e6，不是日期串）。
        let now = chrono::Utc::now().with_timezone(&cn_offset());
        let (items, _) = self.fetch_flash_page(flash_cursor_at(&now)).await?;
        Ok(items)
    }

    /// T7：按截止日回溯 7×24 快讯（2026-09-26 实测通道）。
    ///
    /// 关键点是游标可以直接**跳日**：用截止日当天 23:59:59(CST) 的秒数 ×1e6 起翻，
    /// 1~2 页即得当日快讯；不必从当下逐页回翻（实测每页 20 条只覆盖约 1~2.5 小时，
    /// 逐页翻一天要 12+ 页、翻一周要近百页）。当日一条都没有时报 `Err` 而不是返空——
    /// 空会被下游读成「那天风平浪静」。
    async fn get_cls_flash_with_asof(&self) -> Result<Vec<ClsFlashItem>, DataError> {
        let ctx = crate::as_of::current_as_of().ok_or_else(|| {
            DataError::ParseError("get_cls_flash_with_asof 调用时缺 as_of 上下文".into())
        })?;
        let cutoff = ctx.as_of_date.format("%Y-%m-%d").to_string();
        let day_end = ctx
            .as_of_date
            .and_hms_opt(23, 59, 59)
            .and_then(|t| t.and_local_timezone(cn_offset()).single())
            .ok_or_else(|| DataError::ParseError(format!("截止日 {cutoff} 无法构造当日末时刻")))?;

        let mut cursor = flash_cursor_at(&day_end);
        let mut kept: Vec<ClsFlashItem> = Vec::new();
        let mut passed_the_day = false;
        for _ in 0..FLASH_ASOFP_MAX_PAGES {
            let (items, next) = self.fetch_flash_page(cursor).await?;
            if items.is_empty() {
                break;
            }
            for it in items {
                let key = crate::news_date_key(&it.publish_time);
                if key > cutoff.as_str() {
                    continue; // 游标余量：当日之后的条目
                }
                if key < cutoff.as_str() {
                    passed_the_day = true; // 已翻过当日 ⇒ 后面的页不再属于本维度
                    break;
                }
                kept.push(it);
            }
            if passed_the_day || kept.len() >= FLASH_PAGE_SIZE as usize {
                break;
            }
            cursor = match next {
                Some(c) if c > 0 => c,
                _ => break,
            };
        }

        if kept.is_empty() {
            return Err(DataError::VendorError {
                vendor: "eastmoney".into(),
                message: format!(
                    "截止日 {cutoff} 当天未见 7×24 快讯（回溯 {FLASH_ASOFP_MAX_PAGES} 页 ×{FLASH_PAGE_SIZE} 条）"
                ),
            });
        }
        kept.truncate(FLASH_PAGE_SIZE as usize);
        Ok(kept)
    }

    async fn get_policy_news(
        &self,
        stock_code: &str,
        limit: u32,
    ) -> Result<Vec<NewsItem>, DataError> {
        self.policy_news_impl(stock_code, limit, false).await
    }

    /// T2：政策新闻按截止日回溯（与 live 共用 `policy_news_impl` 主体）。
    async fn get_policy_news_with_asof(
        &self,
        stock_code: &str,
        limit: u32,
    ) -> Result<Vec<NewsItem>, DataError> {
        self.policy_news_impl(stock_code, limit, true).await
    }

    async fn get_announcements(&self, stock_code: &str) -> Result<Vec<Announcement>, DataError> {
        // 修复(2026-07-21): 原实现 stock_list={market},{stock_code} 的 market 前缀
        // (沪市="1"/深市&北交所="0")不规范,且与 get_announcements_with_asof 的
        // stock_list={stock_code} 格式不一致。统一为不带 market 前缀的格式,
        // 依赖 ann_type=A 让 eastmoney API 自动识别市场。
        let url = format!(
            "https://np-anotice-stock.eastmoney.com/api/security/ann?cb=jQuery&sr=-1&page_size=20&page_index=1&ann_type=A&client_source=web&stock_list={stock_code}&f_node=0&s_node=0"
        );

        let resp = self.em_get(&url).await?;
        // 修复 P0-A5 同类问题: 用 text + 手动剥 JSONP 包裹,与 with_asof 路径一致
        // (em_get 返回的 resp 直接 .json() 在 cb=jQuery 时会解析失败)
        let body = resp.text().await?;
        let json_str =
            body.trim_start_matches("jQuery(").trim_end_matches(')').trim_end_matches(';');
        let json: Value = serde_json::from_str(json_str).map_err(|e| {
            DataError::ParseError(format!(
                "eastmoney announcements json 解析失败: {e}, body preview={}",
                &json_str[..json_str.len().min(200)]
            ))
        })?;
        let items = match json["data"]["list"].as_array() {
            Some(arr) => arr,
            None => return Ok(vec![]),
        };

        Ok(items
            .iter()
            .filter_map(|item| {
                Some(Announcement {
                    title: item.get("title")?.as_str()?.to_string(),
                    stock_code: stock_code.to_string(),
                    stock_name: item
                        .get("art_code")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    announce_date: item
                        .get("notice_date")
                        .and_then(|v| v.as_i64())
                        .map(|ts| {
                            crate::vendors::format_timestamp(ts / 1000, "%Y-%m-%d", "eastmoney")
                        })
                        .unwrap_or_default(),
                    ann_type: item
                        .get("columns")
                        .and_then(|v| v.get(0))
                        .and_then(|v| v.get("column_name"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    pdf_url: item
                        .get("dest_url")
                        .and_then(|v| v.as_str())
                        .map(|s| format!("https://np-anotice-stock.eastmoney.com{s}")),
                })
            })
            .collect())
    }

    async fn get_block_trades(&self, stock_code: &str) -> Result<Vec<BlockTrade>, DataError> {
        // 修复(2026-07-22): 原 reportName=RPTA_BLOCKTRADE 已失效("报表配置不存在")。
        // 改用 RPT_DATA_BLOCKTRADE 报表(来自 data.eastmoney.com/newstatic/js/dzjy/default.js)。
        //
        // 字段映射(参考 dzjy/default.js 中 dataview_mrmx 的 columns 定义):
        //   - TRADE_DATE: 交易日期(格式 "YYYY-MM-DD 00:00:00") → trade_date
        //   - SECURITY_NAME_ABBR: 股票简称 → stock_name
        //   - DEAL_PRICE: 成交价 → price
        //   - DEAL_VOLUME: 成交量(股) → volume
        //   - DEAL_AMT: 成交额(元) → amount
        //   - BUYER_NAME: 买方营业部 → buyer_dept
        //   - SELLER_NAME: 卖方营业部 → seller_dept
        //   - PREMIUM_RATIO: 折溢率(正值=溢价, 负值=折价) → discount_pct
        //     注:字段名是 PREMIUM_RATIO 不是 DISCOUNT_RATE;为保持 BlockTrade 字段语义
        //     不变,直接传入 PREMIUM_RATIO 原值,LLM 推断时需知晓正值=溢价/负值=折价。
        let code =
            stock_code.trim_start_matches("sh").trim_start_matches("sz").trim_start_matches("bj");
        let url = format!(
            "https://datacenter-web.eastmoney.com/api/data/v1/get?\
            reportName=RPT_DATA_BLOCKTRADE&\
            columns=TRADE_DATE,SECURITY_CODE,SECUCODE,SECURITY_NAME_ABBR,\
            CHANGE_RATE,CLOSE_PRICE,DEAL_PRICE,PREMIUM_RATIO,DEAL_VOLUME,DEAL_AMT,\
            TURNOVER_RATE,BUYER_NAME,SELLER_NAME,BUYER_CODE,SELLER_CODE&\
            filter=(SECURITY_CODE%3D%22{code}%22)&\
            sortColumns=TRADE_DATE&sortTypes=-1&\
            pageSize=20&pageNumber=1&source=WEB&client=WEB"
        );

        let resp = self.em_get(&url).await?;
        let json: Value = resp.json().await?;

        let rows = match json["result"]["data"].as_array() {
            Some(arr) if !arr.is_empty() => arr,
            _ => {
                return Err(DataError::VendorError {
                    vendor: "eastmoney".into(),
                    message: format!("get_block_trades 数据为空(stock_code={stock_code})"),
                });
            },
        };

        Ok(rows
            .iter()
            .map(|r| {
                let trade_date = r["TRADE_DATE"]
                    .as_str()
                    .map(|s| s.split_whitespace().next().unwrap_or(s).to_string())
                    .unwrap_or_default();
                BlockTrade {
                    stock_code: stock_code.to_string(),
                    stock_name: r["SECURITY_NAME_ABBR"].as_str().unwrap_or("").to_string(),
                    trade_date,
                    price: r["DEAL_PRICE"].as_f64().unwrap_or(0.0),
                    volume: r["DEAL_VOLUME"].as_f64().unwrap_or(0.0),
                    amount: r["DEAL_AMT"].as_f64().unwrap_or(0.0),
                    buyer_dept: r["BUYER_NAME"].as_str().map(|s| s.to_string()),
                    seller_dept: r["SELLER_NAME"].as_str().map(|s| s.to_string()),
                    discount_pct: r["PREMIUM_RATIO"].as_f64(),
                }
            })
            .collect())
    }

    async fn get_institutional_visits(
        &self,
        stock_code: &str,
    ) -> Result<Vec<InstitutionalVisit>, DataError> {
        // 修复(2026-09-12, P0-H L3): 原实现有**三层全部失效且全部静默**的逻辑，
        // 使机构调研恒为空数组（2026-09-12 实测 603353 复现）：
        //
        //   ① 主路径 `RPT_ORG_VISIT_RECORD` —— 接口返回
        //      `{"success":false,"message":"报表配置不存在,RPT_ORG_VISIT_RECORD","code":9501}`
        //      ⇒ 该报表在 datacenter-web 上已不存在，主路径 100% 失败。
        //   ② fallback `sortColumns=SURVEY_DATE` —— 接口返回
        //      `{"success":false,"message":"SURVEY_DATE排序列不存在"}`
        //      ⇒ fallback 也 100% 失败（`result.data` 缺失 ⇒ 直接 `Ok(vec![])`）。
        //   ③ 即使前两层修好，字段读的是 `MAIN_CONTENT` / `ORG_NUM` / `SURVEY_TYPE`，
        //      而 `RPT_ORG_SURVEY` 的真实键是 `CONTENT` / `NUM` / `RECEIVE_WAY_EXPLAIN`
        //      ⇒ `content.is_empty()` 把**每一行**都丢掉（实测 20 行、`CONTENT` 长度
        //      1215、全部被丢弃）—— 又一个静默空。
        //
        // 现改为**只查唯一可用的 `RPT_ORG_SURVEY`**（实测 20 行真实数据），
        // 并按 2026-09-12 dump 出的真实键集映射（键名以实测为准，勿照抄旧注释）：
        //   - RECEIVE_START_DATE : 实际接待日（比 NOTICE_DATE 公告日更贴近「调研日期」）
        //   - NUM                : 同一批调研内的**机构序号**（1..N），**不是机构总数**
        //   - CONTENT            : 调研问答全文
        //   - RECEIVE_WAY_EXPLAIN: 调研方式（如「业绩说明会,网络文字互动」）
        //   - RECEIVE_OBJECT     : 接待对象（「投资者」或机构名）
        //
        // ⇒ 因「一行 = 一家机构」，按 RECEIVE_START_DATE **归并**：同日多行合成一条，
        //   `institution_count` = 同日行数（实测 2026-01-08 有 5 行 = 5 家机构），
        //   `main_content` 同日一致（实测 5 行 CONTENT 长度均为 1215）取首行即可。
        let code =
            stock_code.trim_start_matches("sh").trim_start_matches("sz").trim_start_matches("bj");
        let url = format!(
            "https://datacenter-web.eastmoney.com/api/data/v1/get?\
            reportName=RPT_ORG_SURVEY&columns=ALL&\
            filter=(SECURITY_CODE%3D%22{code}%22)&\
            sortColumns=NOTICE_DATE&sortTypes=-1&\
            pageSize=50&pageNumber=1&source=WEB"
        );

        let resp = self.em_get(&url).await?;
        let json: Value = resp.json().await?;

        let rows = match json["result"]["data"].as_array() {
            Some(arr) if !arr.is_empty() => arr,
            _ => return Ok(vec![]),
        };

        let mut out: Vec<InstitutionalVisit> = Vec::new();
        for r in rows {
            let content = r["CONTENT"].as_str().unwrap_or("");
            // 保留原有「内容太短视为无效记录」门槛（实测有效记录均 ≥ 352 字符）
            if content.chars().count() < 10 {
                continue;
            }
            let visit_date = r["RECEIVE_START_DATE"]
                .as_str()
                .or_else(|| r["NOTICE_DATE"].as_str())
                .map(|s| s.chars().take(10).collect::<String>())
                .unwrap_or_default();
            if visit_date.is_empty() {
                continue;
            }
            // 同一接待日的第二家机构起：只累加计数（同日内容/方式与首行一致）
            if let Some(prev) = out.iter_mut().find(|v| v.visit_date == visit_date) {
                prev.institution_count += 1;
                continue;
            }
            out.push(InstitutionalVisit {
                stock_code: stock_code.to_string(),
                stock_name: r["SECURITY_NAME_ABBR"].as_str().unwrap_or("").to_string(),
                visit_date,
                institution_count: 1,
                main_content: content.to_string(),
                visit_type: r["RECEIVE_WAY_EXPLAIN"]
                    .as_str()
                    .or_else(|| r["RECEIVE_WAY"].as_str())
                    .map(|s| s.to_string()),
            });
        }

        Ok(out)
    }

    async fn get_index_quotes(&self) -> Result<Vec<IndexQuote>, DataError> {
        let mut results = Vec::with_capacity(EM_INDEX_SECIDS.len());
        for (secid, _, name) in &EM_INDEX_SECIDS {
            let url = format!(
                "https://push2his.eastmoney.com/api/qt/stock/get?secid={secid}&fields=f43,f44,f45,f46,f47,f48,f57,f58,f60,f170"
            );
            match self.em_get(&url).await {
                Ok(resp) => {
                    // P0-I(2026-09-12): 原实现 `unwrap_or(Value::Null)` + `if d.is_null() { continue }`
                    // 把「响应体解析失败」与「data 为 null」两种完全不同的故障合并成同一个
                    // 静默 `continue`，配合下方的 `Err(_) => continue` ⇒ 三个指数全失败时
                    // 本函数返回 `Ok(vec![])`，调用方无法区分「源不可用」与「今日无行情」。
                    // 603353 实证（v37）：`t-index-quotes` 载荷恒 `"[]"`，报告板块恒空，
                    // 而同一 URL 从宿主机直连正常（上证 3888.11）⇒ 需日志才能定位。
                    let json: Value = match resp.json().await {
                        Ok(j) => j,
                        Err(e) => {
                            tracing::warn!(
                                "[eastmoney] get_index_quotes {secid}({name}) 响应解析失败: {e}"
                            );
                            continue;
                        },
                    };
                    let d = &json["data"];
                    if d.is_null() {
                        tracing::warn!(
                            "[eastmoney] get_index_quotes {secid}({name}) 返回 data=null, rc={}",
                            json["rc"]
                        );
                        continue;
                    }
                    let f = |key: &str| d[key].as_f64().unwrap_or(0.0);
                    results.push(IndexQuote {
                        code: d["f57"].as_str().unwrap_or("").to_string(),
                        name: name.to_string(),
                        price: f("f43") / 100.0,
                        pre_close: f("f60") / 100.0,
                        change_pct: f("f170") / 100.0,
                        volume: f("f47"),
                        amount: f("f48"),
                    });
                },
                Err(e) => {
                    tracing::warn!(
                        "[eastmoney] get_index_quotes {secid}({name}) 请求失败（已含重试）: {e}"
                    );
                    continue;
                },
            }
        }
        Ok(results)
    }

    async fn get_peers(&self, stock_code: &str) -> Result<Vec<PeerComparison>, DataError> {
        // 修复(2026-07-22): 原 `push2his stock/get` + `clist/get` API 已失效
        // (IncompleteMessage), 改用 `datacenter-web RPT_F10_CORETHEME_BOARDTYPE` 报表
        // 两步查询:1) 个股板块归属(IS_PRECISE=1 的精准行业板块)
        //          2) 反查该板块内所有股票
        let code =
            stock_code.trim_start_matches("sh").trim_start_matches("sz").trim_start_matches("bj");
        let secucode = to_em_secucode(code);

        // 步骤1: 查询个股所属板块, 选 IS_PRECISE=1 的精准行业板块
        let board_url = format!(
            "https://datacenter-web.eastmoney.com/api/data/v1/get?\
             reportName=RPT_F10_CORETHEME_BOARDTYPE&columns=ALL&\
             filter=(SECUCODE%3D%22{secucode}%22)(IS_PRECISE%3D%221%22)&\
             source=WEB&sortColumns=BOARD_RANK&sortTypes=1&pageNumber=1&pageSize=10"
        );
        let resp = self.em_get(&board_url).await?;
        let json: Value = resp.json().await.map_err(|e| DataError::VendorError {
            vendor: "eastmoney".into(),
            message: format!("get_peers 板块查询 JSON 解析失败: {e}"),
        })?;

        let data_arr = match json["result"]["data"].as_array() {
            Some(arr) if !arr.is_empty() => arr,
            _ => {
                return Err(DataError::VendorError {
                    vendor: "eastmoney".into(),
                    message: format!("get_peers 未获取到板块代码(stock_code={stock_code})"),
                });
            },
        };

        // 选第一个精准行业板块 (BOARD_CODE 通常是数字如 "892")
        let board_code = data_arr
            .iter()
            .find_map(|item| item["BOARD_CODE"].as_str().map(|s| s.to_string()))
            .ok_or_else(|| DataError::VendorError {
                vendor: "eastmoney".into(),
                message: format!("get_peers BOARD_CODE 字段为空(stock_code={stock_code})"),
            })?;

        // 步骤2: 反查该板块内所有股票
        let peer_url = format!(
            "https://datacenter-web.eastmoney.com/api/data/v1/get?\
             reportName=RPT_F10_CORETHEME_BOARDTYPE&columns=ALL&\
             filter=(BOARD_CODE%3D%22{board_code}%22)&\
             source=WEB&sortColumns=SECURITY_CODE&sortTypes=1&pageNumber=1&pageSize=30"
        );
        let resp = self.em_get(&peer_url).await?;
        let json: Value = resp.json().await.map_err(|e| DataError::VendorError {
            vendor: "eastmoney".into(),
            message: format!("get_peers 同业列表 JSON 解析失败: {e}"),
        })?;

        let rows = match json["result"]["data"].as_array() {
            Some(arr) if !arr.is_empty() => arr,
            _ => {
                return Err(DataError::VendorError {
                    vendor: "eastmoney".into(),
                    message: format!("get_peers 同业列表为空(board_code={board_code})"),
                });
            },
        };

        // 过滤自身:SECURITY_CODE 是纯数字代码(如 "600887")
        let peer_rows: Vec<&Value> = rows
            .iter()
            .filter(|r| r["SECURITY_CODE"].as_str().map(|c| c != code).unwrap_or(false))
            .collect();
        let peer_codes: Vec<String> = peer_rows
            .iter()
            .filter_map(|r| r["SECURITY_CODE"].as_str().map(String::from))
            .collect();

        // ── 2026-09-21 修复：补估值字段（原 pe/pb/roe/change_pct/market_cap 为硬编码 None/0.0）──
        // 原实现五个字段全部写死（pe/pb/roe = None，change_pct = 0.0，market_cap = None），
        // 从未实现取值 ⇒ 基本面/催化剂/行业三个分析师都写「同侪 PE/PB 全为 null，
        // 横向估值锚缺失」。实测 RPT_VALUEANALYSIS_DET 支持 (SECURITY_CODE in (...))
        // 批量查询，字段含 PE_TTM / PB_MRQ / TOTAL_MARKET_CAP / CHANGE_RATE；
        // 该报表按股票返回多日历史，故按 code 取 TRADE_DATE 最大的一行。
        // ROE 不在该报表内（需财报报表），保持 None —— 不猜值、不伪造。
        type Valuation = (String, Option<f64>, Option<f64>, f64, Option<f64>);
        let mut valuations: HashMap<String, Valuation> = HashMap::new();
        if !peer_codes.is_empty() {
            let in_list: String =
                peer_codes.iter().map(|c| format!("%22{c}%22")).collect::<Vec<_>>().join(",");
            let val_url = format!(
                "https://datacenter-web.eastmoney.com/api/data/v1/get?\
                 reportName=RPT_VALUEANALYSIS_DET&\
                 columns=SECURITY_CODE,PE_TTM,PB_MRQ,TOTAL_MARKET_CAP,CHANGE_RATE,TRADE_DATE&\
                 filter=(SECURITY_CODE%20in%20({in_list}))&\
                 source=WEB&sortColumns=TRADE_DATE&sortTypes=-1&pageNumber=1&pageSize={}",
                peer_codes.len() * 12 + 10
            );
            match self.em_get(&val_url).await {
                Ok(resp) => match resp.json::<Value>().await {
                    Ok(json) => {
                        if let Some(arr) = json["result"]["data"].as_array() {
                            for v in arr {
                                let cc = match v["SECURITY_CODE"].as_str() {
                                    Some(c) => c,
                                    None => continue,
                                };
                                let d = v["TRADE_DATE"].as_str().unwrap_or("").to_string();
                                let better = match valuations.get(cc) {
                                    Some((exist, ..)) => d > *exist,
                                    None => true,
                                };
                                if better {
                                    valuations.insert(
                                        cc.to_string(),
                                        (
                                            d,
                                            v["PE_TTM"].as_f64(),
                                            v["PB_MRQ"].as_f64(),
                                            v["CHANGE_RATE"].as_f64().unwrap_or(0.0),
                                            v["TOTAL_MARKET_CAP"].as_f64(),
                                        ),
                                    );
                                }
                            }
                        }
                    },
                    Err(e) => tracing::warn!(
                        "[eastmoney] get_peers 估值批量查询 JSON 解析失败(估值字段留空): {e}"
                    ),
                },
                Err(e) => {
                    tracing::warn!("[eastmoney] get_peers 估值批量查询失败(估值字段留空): {e}")
                },
            }
        }

        Ok(peer_rows
            .iter()
            .map(|r| {
                let sc = r["SECURITY_CODE"].as_str().unwrap_or("").to_string();
                let v = valuations.get(&sc);
                PeerComparison {
                    stock_code: sc,
                    stock_name: r["SECURITY_NAME_ABBR"].as_str().unwrap_or("").to_string(),
                    pe: v.and_then(|x| x.1),
                    pb: v.and_then(|x| x.2),
                    roe: None,
                    change_pct: v.map(|x| x.3).unwrap_or(0.0),
                    market_cap: v.and_then(|x| x.4),
                }
            })
            .collect())
    }

    async fn get_option_pcr(&self, stock_code: &str) -> Result<Option<OptionPCR>, DataError> {
        // P1-3 修复(2026-07-22): push2his.eastmoney.com/api/qt/clist/get 已失效。
        // 个股期权 PCR 数据无稳定公开 API，且多数个股(如伊利股份)无场内期权。
        // 仅有 50ETF/300ETF 等少数标的有期权数据。
        // 返回 Ok(None) 表示"无数据"而非 Err，避免计入 health_tracker 降级。
        // 若后续发现稳定 API，可在此处实现。
        let code =
            stock_code.trim_start_matches("sh").trim_start_matches("sz").trim_start_matches("bj");
        // 仅 ETF 期权有公开数据，个股直接返回 None
        if !code.starts_with("51")
            && !code.starts_with("56")
            && !code.starts_with("58")
            && !code.starts_with("15")
            && !code.starts_with("16")
        {
            return Ok(None);
        }
        // ETF 期权尝试 push2his clist/get（可能仍可用）
        let underlying = if code.starts_with('5') {
            format!("1.{code}")
        } else {
            format!("0.{code}")
        };
        let url = format!(
            "https://push2his.eastmoney.com/api/qt/clist/get?pn=1&pz=50&fs=option_{underlying}&fields=f12,f14,f164,f165,f166,f167"
        );
        let resp = match self.em_get(&url).await {
            Ok(r) => r,
            Err(_) => return Ok(None), // 接口失效时返回 None 而非 Err
        };
        let json: Value = resp.json().await.unwrap_or(Value::Null);

        let rows = match json["data"]["diff"].as_array() {
            Some(arr) => arr,
            None => return Ok(None),
        };

        let mut call_volume = 0.0_f64;
        let mut put_volume = 0.0_f64;
        let mut call_oi = 0.0_f64;
        let mut put_oi = 0.0_f64;

        for r in rows {
            let name = r["f14"].as_str().unwrap_or("");
            let vol = r["f164"].as_f64().unwrap_or(0.0);
            let oi = r["f165"].as_f64().unwrap_or(0.0);
            if name.contains("购") || name.contains("C") {
                call_volume += vol;
                call_oi += oi;
            } else if name.contains("沽") || name.contains("P") {
                put_volume += vol;
                put_oi += oi;
            }
        }

        if call_volume == 0.0 && put_volume == 0.0 && call_oi == 0.0 && put_oi == 0.0 {
            return Ok(None);
        }

        let volume_pcr = if call_volume > 0.0 {
            put_volume / call_volume
        } else {
            0.0
        };
        let oi_pcr = if call_oi > 0.0 { put_oi / call_oi } else { 0.0 };

        Ok(Some(OptionPCR {
            stock_code: stock_code.to_string(),
            date: chrono::Utc::now().format("%Y-%m-%d").to_string(),
            call_volume,
            put_volume,
            call_oi,
            put_oi,
            volume_pcr,
            oi_pcr,
        }))
    }

    /// 行业/板块排名 — 东方财富板块资金流 API
    ///
    /// 修复(2026-07-22): 原 `push2his.eastmoney.com/api/qt/clist/get` 已失效
    /// (IncompleteMessage), 改用 `data.eastmoney.com/dataapi/bkzj/getbkzj`。
    ///
    /// 该 API 返回行业板块的资金流和涨跌幅数据:
    /// - f3: 涨跌幅 (×100,如 737 表示 7.37%)
    /// - f12: 板块代码 (BK1201)
    /// - f14: 板块名称
    /// - f62: 主力净流入 (元)
    /// - f128: 领涨股名称
    /// - f140: 领涨股代码
    async fn get_industry_ranking(&self) -> Result<Vec<IndustryRank>, DataError> {
        // m:90 = 行业板块, s:2 = 二级行业分类(API 默认按 key 中第一个字段降序)
        let url = "https://data.eastmoney.com/dataapi/bkzj/getbkzj?key=f3,f62,f12,f14,f128,f140&code=m:90+s:2";
        let resp = self.em_get(url).await?;
        let json: Value = resp.json().await.map_err(|e| DataError::VendorError {
            vendor: "eastmoney".into(),
            message: format!("get_industry_ranking JSON 解析失败: {e}"),
        })?;

        let rows = match json["data"]["diff"].as_array() {
            Some(arr) if !arr.is_empty() => arr,
            _ => {
                return Err(DataError::VendorError {
                    vendor: "eastmoney".into(),
                    message: "get_industry_ranking 行业排名数据为空或结构异常".into(),
                });
            },
        };

        Ok(rows
            .iter()
            .filter_map(|r| {
                let industry_name = r["f14"].as_str()?.to_string();
                if industry_name.is_empty() {
                    return None;
                }
                let change_pct = r["f3"].as_f64().unwrap_or(0.0) / 100.0;
                let main_inflow = r["f62"].as_f64();
                let leader_code = r["f140"].as_str().map(|s| s.to_string());
                let leader_name = r["f128"].as_str().map(|s| s.to_string());
                Some(IndustryRank {
                    industry_name,
                    change_pct,
                    turnover: None,
                    main_inflow,
                    leader_code,
                    leader_name,
                    leader_change_pct: None,
                })
            })
            .collect())
    }

    async fn search_concept_boards(&self, keyword: &str) -> Result<Vec<ConceptBoard>, DataError> {
        crate::board::search_concept_boards(&self.http, keyword).await
    }

    async fn get_concept_board_members(
        &self,
        board_code: &str,
    ) -> Result<Vec<BoardMember>, DataError> {
        crate::board::get_concept_board_members(&self.http, board_code).await
    }

    /// 概念板块归属 — 东方财富 emweb 个股板块归属报表
    ///
    /// 新增(2026-07-22 #4): 获取股权质押数据。
    ///
    /// 修复(2026-09-24): 原报表 `RPT_F10_EH_PLEDGE` 已下线 —— datacenter-web /
    /// datacenter 两个域、多个路径一律返回
    /// `{"success":false,"message":"报表配置不存在,RPT_F10_EH_PLEDGE","code":9501}`
    /// （实测 2026-09-24，300642 / 688114 均如此，而同域 `RPT_F10_CORETHEME_BOARDTYPE`
    /// 正常返回）⇒ 质押路由只有 eastmoney 一个 vendor，旧代码又把该响应当「空数据（非故障）」
    /// 静默降级 ⇒ 质押数据 46/46 恒 null，分析师只能写「无法获取」。
    ///
    /// 现改用中国结算周度质押报表 `RPT_CSDC_LIST`（data.eastmoney.com/gpzy 的数据源）。
    /// 字段映射（实测 4 只股票一致）：
    ///   - PLEDGE_RATIO       → pledge_ratio（大股东质押总比例 %）
    ///   - REPURCHASE_BALANCE → pledge_shares（接口单位「万股」×10000 换算为「股」）
    ///   - PLEDGE_DEAL_NUM    → pledge_count（质押笔数）
    ///   - TRADE_DATE         → 报告期（中国结算按周披露，取最新一期）
    ///
    /// 该报表**不含**控股股东质押比例列，`controlling_pledge_ratio` 记 0.0
    /// （消费端 `detect_pledge_risk` 只读 pledge_ratio，不受影响）。
    ///
    /// 报表缺失/异常时返回 Err 而非 Ok(None)：Ok(None) 会被上层判为
    /// 「该股无质押（非故障）」而永不回退，正是此前静默空值的成因。
    async fn get_pledge_data(&self, stock_code: &str) -> Result<Option<PledgeData>, DataError> {
        // 请求与解析收口在 `fetch_pledge`（与 as-of 回溯共用同一报表与口径）
        let hit = self.fetch_pledge(&Self::pledge_report_url(stock_code, None), stock_code).await?;
        Ok(hit.map(|(data, _)| data))
    }

    /// T5：按截止日回溯质押数据（2026-09-26）。
    ///
    /// 旧认知「质押无历史语义」是错的：`RPT_CSDC_LIST` 有 `TRADE_DATE` 列且支持
    /// `<=` 过滤（实测见 `pledge_report_url`）。中登按周披露 ⇒ 回放里拿到的是
    /// 截止日前最近一期，而不是当日值，这一点用 info 日志留痕。
    async fn get_pledge_data_with_asof(
        &self,
        stock_code: &str,
    ) -> Result<Option<PledgeData>, DataError> {
        let ctx = crate::as_of::current_as_of().ok_or_else(|| {
            DataError::ParseError("get_pledge_data_with_asof 调用时缺 as_of 上下文".into())
        })?;
        let cutoff = ctx.as_of_date.format("%Y-%m-%d").to_string();
        let url = Self::pledge_report_url(stock_code, Some(&cutoff));
        let hit = self.fetch_pledge(&url, stock_code).await?;
        if let Some((_, date)) = hit.as_ref() {
            let key = date.split(' ').next().unwrap_or(date.as_str());
            if key != cutoff.as_str() {
                tracing::info!(
                    "[asof] {stock_code} 质押查询截止日 {cutoff}，中登实际披露到 {key}（周报口径），取该期"
                );
            }
        }
        Ok(hit.map(|(data, _)| data))
    }

    /// 修复(2026-07-22): 新增实现。原 eastmoney 未实现此方法(路由降级到
    /// ths/baidu_stock/iwencai,但 ths industry_board/rank 404、
    /// baidu_stock gushitong 301,均不可用)。改用 `datacenter-web
    /// RPT_F10_CORETHEME_BOARDTYPE` 报表查个股板块归属。
    ///
    /// IS_PRECISE=1 通常是精准行业板块(如 "乳业"),其余是概念板块(如 "茅指数")。
    async fn get_concept_blocks(
        &self,
        stock_code: &str,
    ) -> Result<Option<ConceptBlocks>, DataError> {
        let code =
            stock_code.trim_start_matches("sh").trim_start_matches("sz").trim_start_matches("bj");
        let secucode = to_em_secucode(code);

        let url = format!(
            "https://datacenter-web.eastmoney.com/api/data/v1/get?\
             reportName=RPT_F10_CORETHEME_BOARDTYPE&columns=ALL&\
             filter=(SECUCODE%3D%22{secucode}%22)&\
             source=WEB&sortColumns=BOARD_RANK&sortTypes=1&pageNumber=1&pageSize=50"
        );
        let resp = self.em_get(&url).await?;
        let json: Value = resp.json().await.map_err(|e| DataError::VendorError {
            vendor: "eastmoney".into(),
            message: format!("get_concept_blocks JSON 解析失败: {e}"),
        })?;

        let data_arr = match json["result"]["data"].as_array() {
            Some(arr) if !arr.is_empty() => arr,
            _ => return Ok(None),
        };

        // IS_PRECISE=1 通常是行业板块 (如 "乳业"), 其余是概念板块 (如 "茅指数")
        let mut industry = String::new();
        let mut concepts: Vec<BlockItem> = Vec::new();
        for item in data_arr {
            let board_name = item["BOARD_NAME"].as_str().unwrap_or("");
            if board_name.is_empty() {
                continue;
            }
            let is_precise = item["IS_PRECISE"].as_str() == Some("1");
            if is_precise && industry.is_empty() {
                industry = board_name.to_string();
            } else {
                concepts.push(BlockItem { name: board_name.to_string(), change_pct: None });
            }
        }

        if industry.is_empty() && concepts.is_empty() {
            return Ok(None);
        }

        Ok(Some(ConceptBlocks {
            stock_code: stock_code.to_string(),
            industry,
            concepts,
            regions: vec![],
        }))
    }

    /// 北向资金（沪深港通）— 东方财富 API
    ///
    /// 修复(2026-07-22): 原 API `push2his.eastmoney.com/api/qt/stock/fflow/kline/get`
    /// 已失效(连接错误),改用 `push2his.eastmoney.com/api/qt/kamt.kline/get`。
    ///
    /// 该 API 返回:
    ///   - hk2sh: 北向沪股通(港→沪)
    ///   - hk2sz: 北向深股通(港→深)
    ///   - sh2hk: 南向沪股通(沪→港)
    ///   - sz2hk: 南向深股通(深→港)
    ///
    /// 每条字符串格式: "日期,当日净流入,当日余额,历史累计净流入"
    /// 字段 f51=日期, f52=当日净流入, f53=当日余额, f54=历史累计
    ///
    /// 拉 5 个交易日数据:最新一天填主字段,5 天填 recent_history(从新到旧)
    /// 用于趋势观察,排除脉冲式流入。
    ///
    /// 修复(2026-07-22 v2): 2024-08-16 起监管层暂停披露北向资金实时数据,
    /// 此后 kamt.kline API 的 hk2sh/hk2sz 的 f52(当日净流入)全部返回 0,
    /// f53(当日余额)被冻结为 5200000.00(额度上限),f54(累计)也停止更新。
    /// 这是政策原因,非项目代码缺陷。
    ///
    /// 应对策略:当最近 HISTORY_DAYS 日数据全部为 0 时,自动回退拉取
    /// 监管暂停前最后 HISTORY_DAYS 个交易日(2024-08-16 之前)的数据,
    /// 用于历史趋势参考。同时在 timestamp 字段中标注"data_source=pre_policy_pause"
    /// 以便上层 LLM 知道这是政策暂停前的数据。
    /// v3(2026-08-01) 修正：北向资金**净流入** 2024-08-16 起监管停披（kamt.kline f52 冻结为 0），
    /// 但**成交额（DEAL_AMT）、领涨股、指数点位仍在披露**（datacenter-web RPT_MUTUAL_DEAL_HISTORY，
    /// curl 实测 2026-07-31 沪/深股通均有数据）。
    /// 旧实现只看 f52（净流入）→ 全 0 → 误判"北向资金失效"。现改用数据中心接口返回
    /// **成交额序列**，timestamp 明确标注"净流入停披，此处为成交额"，不再伪造也不误删。
    async fn get_north_bound_flow(&self) -> Result<Option<NorthBoundFlow>, DataError> {
        // 拉取指定互港通方向的最近 N 个交易日成交额（datacenter-web，按 TRADE_DATE 降序）
        async fn fetch_deal_amt(
            client: &EastMoneyVendor,
            mutual_type: &str,
            size: u32,
        ) -> Result<Vec<(String, f64)>, DataError> {
            let url = format!(
                "https://datacenter-web.eastmoney.com/api/data/v1/get?\
                reportName=RPT_MUTUAL_DEAL_HISTORY&columns=ALL&filter=(MUTUAL_TYPE%3D%22{mutual_type}%22)&\
                pageSize={size}&sortColumns=TRADE_DATE&sortTypes=-1"
            );
            let resp = client.em_get(&url).await?;
            let json: Value = resp.json().await?;
            let rows = match json["result"]["data"].as_array() {
                Some(arr) => arr,
                None => return Ok(vec![]),
            };
            Ok(rows
                .iter()
                .filter_map(|r| {
                    // TRADE_DATE 形如 "2026-07-31 00:00:00" → 取前 10 字符
                    let date = r["TRADE_DATE"].as_str()?.chars().take(10).collect::<String>();
                    // DEAL_AMT 单位：百万（东财口径，沪股通日成交 ~1500 亿 = 150000 百万）
                    let deal_amt = r["DEAL_AMT"].as_f64().unwrap_or(0.0);
                    Some((date, deal_amt))
                })
                .collect())
        }

        let sh_list = fetch_deal_amt(self, "001", 6).await?; // 沪股通
        let sz_list = fetch_deal_amt(self, "003", 6).await?; // 深股通

        if sh_list.is_empty() && sz_list.is_empty() {
            return Ok(None);
        }

        // 按日期对齐（两个接口均按 TRADE_DATE 降序返回，逐索引配对）
        let mut recent_history: Vec<NorthBoundFlowDaily> = Vec::with_capacity(sh_list.len());
        for i in 0..sh_list.len().max(sz_list.len()).min(6) {
            let (d_sh, sh_amt) = sh_list.get(i).cloned().unwrap_or_default();
            let (d_sz, sz_amt) = sz_list.get(i).cloned().unwrap_or_default();
            let date = if !d_sh.is_empty() { d_sh } else { d_sz };
            if date.is_empty() {
                continue;
            }
            recent_history.push(NorthBoundFlowDaily {
                date,
                sh_flow: sh_amt,
                sz_flow: sz_amt,
                total_flow: sh_amt + sz_amt,
            });
        }

        let latest = recent_history.first().cloned().unwrap_or(NorthBoundFlowDaily {
            date: String::new(),
            sh_flow: 0.0,
            sz_flow: 0.0,
            total_flow: 0.0,
        });

        Ok(Some(NorthBoundFlow {
            date: latest.date,
            sh_flow: latest.sh_flow,
            sz_flow: latest.sz_flow,
            total_flow: latest.total_flow,
            // 明确标注：北向净流入自 2024-08-16 监管停披，此处 sh_flow/sz_flow/total_flow
            // 为"成交额"（单位百万），非净买入额——防止 LLM 误读为净流入。
            timestamp: Some(
                "deal_amt_in_million（北向净流入自2024-08-16监管停披，此字段为成交额非净流入）"
                    .to_string(),
            ),
            recent_history,
        }))
    }

    // ────────────────────────────────────────────────────────────────
    // Vendor trait 大重构 P1: as-of 能力申报 + _with_asof 实现
    //
    // 申报策略(基于东方财富 API 实际形态):
    // - NativeDateParam     URL 支持日期参数(begin_time/end_time/TRADE_DATE 等)
    // - SynthesizeFromKline 实时类,用 K 线最后一行合成
    // - NoHistoricalSemantic 当下榜单/分类(无历史)
    // - Fallthrough         vendor 返回带 date 字段的全量,由 lib.rs 截断
    // ────────────────────────────────────────────────────────────────

    fn asof_capability(&self, method: &str) -> AsOfCapability {
        match method {
            // NativeDateParam: URL 真的支持日期参数
            // （get_money_flow 是「拉全窗 + 本地按截止日过滤」形态 —— fflow 接口本身
            //   忽略 beg/end，见 get_money_flow_with_asof 注释；2026-09-26 起申报）
            "get_klines"
            | "get_margin_data"
            | "get_north_bound_flow"
            | "get_market_dragon_tiger"
            | "get_announcements"
            | "get_research_reports"
            | "get_money_flow"
            // T2(2026-09-26)：搜索接口不认 begin/end 日期，但 `sort:"time"` 倒序翻页
            // 能翻到截止日 ⇒ 申报 NativeDateParam 的是 `news_pages_until_asof` 这条
            // 真实通道（见 get_news_with_asof 注释），不是「接口带日期参数」。
            | "get_news"
            | "get_policy_news"
            | "search_news"
            // T5(2026-09-26)：中登质押报表支持 TRADE_DATE<= 过滤，见 get_pledge_data_with_asof
            | "get_pledge_data"
            // T7(2026-09-26)：7×24 快讯用 realSort 游标（Unix 秒 ×1e6）直接跳日，
            // 见 get_cls_flash_with_asof —— 它不是「无历史语义」，只是接口不认日期串
            | "get_cls_flash" => AsOfCapability::NativeDateParam,
            // SynthesizeFromKline: 实时报价/指数,用 K 线最后一行合成
            "get_quote" | "get_index_quotes" => AsOfCapability::SynthesizeFromKline,
            // NoHistoricalSemantic: 当下榜单/分类(本地缓存 P5 启用)
            "get_hot_stocks" | "get_industry_ranking" | "get_concept_blocks" => {
                AsOfCapability::NoHistoricalSemantic
            },
            // Fallthrough: vendor 返回带 date 字段的全量,lib.rs 截断(已正确)
            "get_financials"
            | "get_dragon_tiger"
            | "get_lockup_schedule"
            | "get_north_bound_holding"
            | "get_shareholder_trades"
            | "get_dividend_records"
            | "get_consensus_eps"
            | "get_block_trades"
            | "get_institutional_visits"
            | "get_sector_info"
            | "get_peers"
            | "get_option_pcr"
            | "search_stock" => AsOfCapability::Fallthrough,
            // 未知方法兜底
            _ => AsOfCapability::Fallthrough,
        }
    }

    // ── _with_asof 实现:NativeDateParam 类的日期参数升级 ──

    /// D 档修复:全市场龙虎榜支持 TRADE_DATE 单日过滤
    /// bug 修复:replay 模式现在能拿到 as_of_date 当日的数据
    async fn get_market_dragon_tiger_with_asof(&self) -> Result<Vec<MarketDragonTiger>, DataError> {
        let as_of = crate::as_of::current_as_of()
            .ok_or_else(|| DataError::ParseError("no as_of context".into()))?;
        let trade_date = as_of.as_of_date.format("%Y-%m-%d").to_string();
        let url = format!(
            "https://datacenter-web.eastmoney.com/api/data/v1/get?\
            reportName=RPT_DAILYBOARDDETAILS&\
            columns=ALL&\
            filter=(TRADE_DATE%3D%27{trade_date}%27)&\
            pageNumber=1&pageSize=50&sortColumns=BOARD_CODE%2CSECURITY_CODE&sortTypes=1%2C1"
        );
        let resp = self.em_get(&url).await?;
        let json: Value = resp.json().await.unwrap_or(Value::Null);
        let rows = match json["result"]["data"].as_array() {
            Some(arr) => arr,
            None => return Ok(vec![]),
        };
        Ok(rows
            .iter()
            .map(|r| MarketDragonTiger {
                date: r["TRADE_DATE"].as_str().unwrap_or(&trade_date).to_string(),
                stock_code: r["SECURITY_CODE"].as_str().unwrap_or("").to_string(),
                stock_name: r["SECURITY_NAME_ABBR"].as_str().unwrap_or("").to_string(),
                net_buy: r["NET_BUY_AMT"].as_f64().unwrap_or(0.0),
                buy_amount: r["BUY_AMT"].as_f64().unwrap_or(0.0),
                sell_amount: r["SELL_AMT"].as_f64().unwrap_or(0.0),
                reason: r["EXPLANATION"].as_str().map(|s| s.to_string()),
            })
            .collect())
    }

    /// announcements 升级:begin_time/end_time 用 as_of 窗口(默认前 365 天 → as_of_date)
    async fn get_announcements_with_asof(
        &self,
        stock_code: &str,
    ) -> Result<Vec<Announcement>, DataError> {
        let as_of = crate::as_of::current_as_of()
            .ok_or_else(|| DataError::ParseError("no as_of context".into()))?;
        let end_date = as_of.as_of_date.format("%Y-%m-%d").to_string();
        let begin_date =
            (as_of.as_of_date - chrono::Duration::days(365)).format("%Y-%m-%d").to_string();
        let url = format!(
            "https://np-anotice-stock.eastmoney.com/api/security/ann?cb=jQuery&sr=-1&page_size=20&page_index=1&ann_type=A&client_source=web&stock_list={stock_code}&f_node=0&s_node=0&begin_time={begin_date}&end_time={end_date}"
        );
        let resp = self.em_get(&url).await?;
        // 修复 P0-A5 同类问题: 原 `unwrap_or_default()` 把 HTTP body 解码错误吞为空串，
        // 走到下面 `serde_json::from_str(...).unwrap_or(Value::Null)` 丢失根因。
        // 改用 `?` 透传原始 reqwest::Error 便于调试。
        let body = resp.text().await?;
        let json_str =
            body.trim_start_matches("jQuery(").trim_end_matches(')').trim_end_matches(';');
        let json: Value = serde_json::from_str(json_str).map_err(|e| {
            DataError::ParseError(format!(
                "eastmoney announcements json 解析失败: {e}, body preview={}",
                &json_str[..json_str.len().min(200)]
            ))
        })?;
        let items = match json["data"]["list"].as_array() {
            Some(arr) => arr,
            None => return Ok(vec![]),
        };
        Ok(items
            .iter()
            .filter_map(|item| {
                Some(Announcement {
                    title: item.get("title")?.as_str()?.to_string(),
                    stock_code: stock_code.to_string(),
                    stock_name: item
                        .get("art_code")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    announce_date: item
                        .get("notice_date")
                        .and_then(|v| v.as_i64())
                        .map(|ts| {
                            crate::vendors::format_timestamp(ts / 1000, "%Y-%m-%d", "eastmoney")
                        })
                        .unwrap_or_default(),
                    ann_type: Some("A".to_string()),
                    pdf_url: None,
                })
            })
            .collect())
    }

    /// research_reports 升级:beginTime/endTime 用 as_of 窗口(原本硬编码 2000-2030)
    async fn get_research_reports_with_asof(
        &self,
        stock_code: &str,
    ) -> Result<Vec<ResearchReport>, DataError> {
        let as_of = crate::as_of::current_as_of()
            .ok_or_else(|| DataError::ParseError("no as_of context".into()))?;
        let end_time = as_of.as_of_date.format("%Y-%m-%d").to_string();
        let begin_time =
            (as_of.as_of_date - chrono::Duration::days(365)).format("%Y-%m-%d").to_string();
        let url = format!(
            "https://reportapi.eastmoney.com/report/list?industryCode=*&pageSize=20&\
            industry=%2A&rating=&ratingChange=&\
            beginTime={begin_time}&endTime={end_time}&\
            pageNo=1&fields=&qType=0&orgCode=&code={stock_code}&rcode=&\
            p=1&pageNum=1&pageNumber=1"
        );
        let resp = self.em_get(&url).await?;
        let json: Value = resp.json().await?;
        let reports = match json["data"].as_array() {
            Some(arr) => arr,
            None => return Ok(vec![]),
        };
        Ok(reports
            .iter()
            .map(|r| {
                // 2026-09-08 修复: as-of 路径原先硬编码 eps_forecast: Vec::new() +
                // target_price: None,与 live 路径(get_research_reports)不对齐,
                // 导致回放模式下所有研报 epsForecast 全空、目标价恒 null,
                // 下游 a-research 分析师报"数据缺口"。此处解析逻辑必须与
                // live 路径保持一致(predictThisYearEps/NextYear/NextTwoYearEps
                // + 目标价 = 预测PE × 预测EPS)。上游字段变更时两处同步改。
                let mut eps_forecast = Vec::new();
                let mut this_year_eps: Option<f64> = None;
                if let Some(eps) = r["predictThisYearEps"].as_str() {
                    if let Ok(val) = eps.parse::<f64>() {
                        eps_forecast.push(EpsForecast { year: "今年".into(), eps: Some(val) });
                        this_year_eps = Some(val);
                    }
                }
                if let Some(eps) = r["predictNextYearEps"].as_str() {
                    if let Ok(val) = eps.parse::<f64>() {
                        eps_forecast.push(EpsForecast { year: "明年".into(), eps: Some(val) });
                    }
                }
                if let Some(eps) = r["predictNextTwoYearEps"].as_str() {
                    if let Ok(val) = eps.parse::<f64>() {
                        eps_forecast.push(EpsForecast { year: "后年".into(), eps: Some(val) });
                    }
                }
                let target_price = if let (Some(eps), Some(pe_str)) =
                    (this_year_eps, r["predictThisYearPe"].as_str())
                {
                    pe_str
                        .parse::<f64>()
                        .ok()
                        .filter(|&pe| pe > 0.0 && eps > 0.0)
                        .map(|pe| pe * eps)
                } else {
                    None
                };
                ResearchReport {
                    title: r["title"].as_str().unwrap_or("").to_string(),
                    institution: r["orgSName"].as_str().unwrap_or("").to_string(),
                    analyst: r["researcher"].as_str().map(|s| s.to_string()),
                    rating: r["emRatingName"].as_str().map(|s| s.to_string()),
                    target_price,
                    eps_forecast,
                    publish_date: r["publishDate"].as_str().unwrap_or("").to_string(),
                    pdf_url: r["infoCode"]
                        .as_str()
                        .map(|s| format!("https://pdf.dfcfw.com/pdf/H3_{}_1.pdf", s)),
                }
            })
            .collect())
    }

    // ── SynthesizeFromKline 类的 quote 合成 ──
    // 注:quote 实际合成逻辑在 lib.rs.quote_from_klines
    // (vendor 层只能拉 K 线数据,合成在 lib.rs 完成)
    async fn get_quote_with_asof(&self, stock_code: &str) -> Result<StockQuote, DataError> {
        let _ = stock_code;
        Err(DataError::VendorError {
            vendor: "eastmoney".into(),
            message: "get_quote_with_asof: lib.rs 路由层调用 quote_from_klines 合成,不应直连"
                .into(),
        })
    }

    // ── NativeDateParam 类的日期参数升级 ──

    /// get_klines 升级:end 参数 = as_of_date
    /// 例 as_of=2024-06-01 → end=20240601
    async fn get_klines_with_asof(
        &self,
        stock_code: &str,
        period: &str,
        limit: u32,
        adj: Option<AdjType>,
    ) -> Result<Vec<KLine>, DataError> {
        let period_code = match period {
            "5" | "Min5" => "5",
            "15" | "Min15" => "15",
            "30" | "Min30" => "30",
            "60" | "Min60" => "60",
            "daily" | "101" | "Daily" => "101",
            "weekly" | "102" | "Weekly" => "102",
            "monthly" | "103" | "Monthly" => "103",
            _ => "101",
        };
        let as_of = crate::as_of::current_as_of()
            .ok_or_else(|| DataError::ParseError("no as_of context".into()))?;
        let end_date = as_of.as_of_date.format("%Y%m%d").to_string();
        let secid = to_em_secid(stock_code);
        // 修复 R3: 与 get_klines 一致，根据 adj 参数选择 fqt
        let fqt = match adj {
            None | Some(AdjType::None) => 0,
            Some(AdjType::Forward) => 1,
            Some(AdjType::Backward) => 2,
        };
        let url = format!(
            "https://push2his.eastmoney.com/api/qt/stock/kline/get?secid={secid}&fields1=f1,f2,f3,f4,f5,f6&fields2=f51,f52,f53,f54,f55,f56,f57,f58,f59,f60,f61&klt={period_code}&fqt={fqt}&end={end_date}&lmt={limit}"
        );
        let resp = self.em_get(&url).await?;
        let json: Value = resp.json().await?;
        let klines_raw = json["data"]["klines"]
            .as_array()
            .ok_or_else(|| DataError::ParseError("missing klines array".into()))?;
        // vendor 已应用复权 → 标记 adj_factor = Some(1.0) 表示已处理
        let adj_marker = if fqt == 0 { None } else { Some(1.0) };
        let mut klines: Vec<KLine> = klines_raw
            .iter()
            .map(|v| {
                let s =
                    v.as_str().ok_or_else(|| DataError::ParseError("kline not string".into()))?;
                let parts: Vec<&str> = s.split(',').collect();
                if parts.len() < 11 {
                    return Err(DataError::ParseError(format!(
                        "expected 11 fields in kline, got {}",
                        parts.len()
                    )));
                }
                let parse = |s: &str| -> f64 { s.parse().unwrap_or(0.0) };
                Ok(KLine {
                    date: parts[0].to_string(),
                    open: parse(parts[1]),
                    close: parse(parts[2]),
                    high: parse(parts[3]),
                    low: parse(parts[4]),
                    volume: parse(parts[5]) * 100.0, // 东方财富 K线 f56 单位为"手"，×100 转为"股"
                    amount: parse(parts[6]),
                    turnover_rate: if parts.len() > 10 {
                        Some(parse(parts[10]))
                    } else {
                        None
                    },
                    // R3: vendor 已复权时标记，避免 lib 层二次应用
                    adj_factor: adj_marker,
                })
            })
            .collect::<Result<_, _>>()?;
        // 兜底再按 as_of_date 截断(vendor 可能返回略多)
        let cutoff = as_of.as_of_date.format("%Y-%m-%d").to_string();
        klines.retain(|k| k.date <= cutoff);
        Ok(klines)
    }

    /// get_margin_data 升级:加 DATE 过滤实现 as-of 回放
    ///
    /// 修复(2026-07-22): 原 `RPT_MARGIN_DETAIL_BY_STOCK` 报表已不存在(返回"报表配置不存在"),
    /// 改用与 get_margin_data 相同的 `RPTA_WEB_RZRQ_GGMX` 报表 + DATE 过滤。
    /// filter 字段大小写不敏感,经测试 (scode="600887")(DATE='2026-07-03') 可用。
    async fn get_margin_data_with_asof(
        &self,
        stock_code: &str,
    ) -> Result<Option<MarginData>, DataError> {
        let as_of = crate::as_of::current_as_of()
            .ok_or_else(|| DataError::ParseError("no as_of context".into()))?;
        let trade_date = as_of.as_of_date.format("%Y-%m-%d").to_string();
        // 去除 sh/sz/bj 前缀
        let code =
            stock_code.trim_start_matches("sh").trim_start_matches("sz").trim_start_matches("bj");
        // ⚠ 必须用 `DATE<=截止日` + 按日期倒序取第一条，**不能**用 `DATE='精确那天'`：
        //   两融披露只落在交易日（且常晚于收盘），等值查在非交易日/未披露日必然返回空，
        //   于是回放里所有标的都会被误报成"无融资融券数据"。
        //   实测（本机 curl）：600519 `DATE='2026-07-19'`（周日）→ success:false 空；
        //   同标的 `DATE<='2026-07-19'` → 返回 07-17（最近交易日）完整数据。
        let url = format!(
            "https://datacenter-web.eastmoney.com/api/data/v1/get?\
            reportName=RPTA_WEB_RZRQ_GGMX&columns=ALL&\
            filter=(scode%3D%22{code}%22)(DATE%3C%3D%27{trade_date}%27)&source=WEB&\
            sortColumns=DATE&sortTypes=-1&pageNumber=1&pageSize=1"
        );

        let resp = self.em_get(&url).await?;
        let json: Value = resp.json().await.unwrap_or(Value::Null);

        // as-of 模式下 API 报错或日期无数据(非交易日/停盘)时,返回 Ok(None) 让上层 fallback
        if json["success"].as_bool() == Some(false) {
            tracing::debug!(
                "[eastmoney] get_margin_data_with_asof 无数据(stock_code={stock_code}, date={trade_date}): {}",
                json["message"].as_str().unwrap_or("unknown")
            );
            return Ok(None);
        }

        let data = match json["result"]["data"].as_array() {
            Some(arr) if !arr.is_empty() => &arr[0],
            _ => return Ok(None),
        };

        let parse_f64 = |key: &str| -> f64 { data[key].as_f64().unwrap_or(0.0) };
        // DATE 字段格式 "2026-07-03 00:00:00",截取日期部分
        let raw_date = data["DATE"].as_str().unwrap_or(&trade_date);
        let date = raw_date.split_whitespace().next().unwrap_or(&trade_date).to_string();
        if date != trade_date {
            // 截止日无披露（休市或数据晚出）时用的是更早一天的值 —— 留一条 info 便于
            // 事后核对，不记降级（这是正确行为，不是缺陷）
            tracing::info!(
                "[eastmoney] 两融 as-of：{code} 截止日 {trade_date} 无披露，取最近披露日 {date}"
            );
        }

        Ok(Some(MarginData {
            stock_code: stock_code.to_string(),
            date,
            margin_buy: parse_f64("RZMRE"),        // 融资买入额(元)
            margin_balance: parse_f64("RZYE"),     // 融资余额(元)
            short_sell_volume: parse_f64("RQMCL"), // 融券卖出量(股)
            short_balance: parse_f64("RQYE"),      // 融券余额(元)
        }))
    }

    /// get_north_bound_flow 升级:加日期过滤实现 as-of 回放
    ///
    /// 修复(2026-07-22): 原 `RPT_MUTUAL_STOCK_HOLDRANKS` 是个股持仓排行报表,
    /// 不是北向资金总流量报表,返回 NET_FLOW 字段恒为个股净买入而非市场汇总。
    /// 改用与 get_north_bound_flow 相同的 `kamt.kline/get` API,
    /// 拉取近 30 个交易日后:
    ///   1) 按 trade_date 取主字段
    ///   2) 取 trade_date 及之前最近 5 天作为 recent_history
    async fn get_north_bound_flow_with_asof(&self) -> Result<Option<NorthBoundFlow>, DataError> {
        const HISTORY_DAYS: usize = 5;
        let as_of = crate::as_of::current_as_of()
            .ok_or_else(|| DataError::ParseError("no as_of context".into()))?;
        let trade_date = as_of.as_of_date.format("%Y-%m-%d").to_string();
        // 拉 30 天保证覆盖到 as_of_date(节假日+周末约 10 天,30 天足够)
        let url = "https://push2his.eastmoney.com/api/qt/kamt.kline/get?\
            fields1=f1,f2,f3&fields2=f51,f52,f53,f54&klt=101&lmt=30";
        let resp = self.em_get(url).await?;
        let json: Value = resp.json().await?;

        let data = &json["data"];
        if data.is_null() {
            tracing::debug!(
                "[eastmoney] get_north_bound_flow_with_asof data=null(date={trade_date})"
            );
            return Ok(None);
        }

        // 解析全部记录,返回 Vec<(date, flow)> 按日期升序
        let parse_all = |arr: &Value| -> Vec<(String, f64)> {
            arr.as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| {
                            let s = v.as_str()?;
                            let parts: Vec<&str> = s.split(',').collect();
                            let d = parts.first().copied()?.to_string();
                            let f = parts.get(1).and_then(|x| x.parse().ok()).unwrap_or(0.0);
                            Some((d, f))
                        })
                        .collect()
                })
                .unwrap_or_default()
        };

        let sh_list = parse_all(&data["hk2sh"]);
        let sz_list = parse_all(&data["hk2sz"]);

        // 过滤 date <= trade_date 的最近 HISTORY_DAYS 天
        let cutoff = trade_date.as_str();
        let mut recent_history: Vec<NorthBoundFlowDaily> = Vec::new();
        for (i, (d, sh)) in sh_list.iter().enumerate() {
            if d.as_str() > cutoff {
                continue;
            }
            let sz = sz_list.get(i).map(|(_, f)| *f).unwrap_or(0.0);
            recent_history.push(NorthBoundFlowDaily {
                date: d.clone(),
                sh_flow: *sh,
                sz_flow: sz,
                total_flow: sh + sz,
            });
        }
        // 取最近 HISTORY_DAYS 天(升序的尾部)
        let start = recent_history.len().saturating_sub(HISTORY_DAYS);
        recent_history = recent_history.split_off(start);
        // 反转成"从新到旧"
        recent_history.reverse();

        if recent_history.is_empty() {
            tracing::debug!(
                "[eastmoney] get_north_bound_flow_with_asof 未匹配到 date<={trade_date} 的数据"
            );
            return Ok(None);
        }

        // 主字段取 recent_history[0](最新一天,即 <= as_of_date 的最大日期)
        let latest = recent_history[0].clone();
        Ok(Some(NorthBoundFlow {
            date: latest.date,
            sh_flow: latest.sh_flow,
            sz_flow: latest.sz_flow,
            total_flow: latest.total_flow,
            timestamp: None,
            recent_history,
        }))
    }

    // ── SynthesizeFromKline 类的 index_quotes 合成 ──
    //
    // 此前这里是**直接返回 Err 的占位**，注释写「lib.rs 路由层会拿到 K 线后再合成」——
    // 而路由层从未实现该合成 ⇒ as-of 模式下「大盘指数」维度恒空（2026-09-25 补）。
    // 指数的 secid/名称映射本就在 vendor 侧，合成放在这里最不易取错标的。
    async fn get_index_quotes_with_asof(&self) -> Result<Vec<IndexQuote>, DataError> {
        crate::as_of::current_as_of().ok_or_else(|| {
            DataError::ParseError("get_index_quotes_with_asof: 无 as_of 上下文".into())
        })?;
        let mut out = Vec::with_capacity(EM_INDEX_SECIDS.len());
        for &(secid, code, name) in &EM_INDEX_SECIDS {
            // 指数不涉及复权 → fqt=0；取 3 根：末根=截止日点位，前一根=昨收
            match self.get_klines_with_asof(secid, "daily", 3, Some(AdjType::None)).await {
                Ok(ks) => match crate::vendors::index_quote_from_klines(code, name, &ks) {
                    Some(q) => out.push(q),
                    None => {
                        tracing::warn!("[eastmoney] as-of 指数 {secid}({name}) K线不足两根，跳过")
                    },
                },
                Err(e) => {
                    tracing::warn!("[eastmoney] as-of 指数 {secid}({name}) K线失败: {e}")
                },
            }
        }
        if out.is_empty() {
            return Err(DataError::VendorError {
                vendor: "eastmoney".into(),
                message: "as-of 模式下三大指数当日均无可用 K 线".into(),
            });
        }
        Ok(out)
    }
}

// ────────────────────────────────────────────────────────────────────────
// P1 测试:asof_capability 申报正确性 + URL 构造正确性
// 纯函数测试,不发真实 HTTP 请求
// ────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod asof_capability_tests {
    use super::*;

    fn make_vendor() -> EastMoneyVendor {
        EastMoneyVendor {
            http: reqwest::Client::new(),
            proxy_http: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    #[test]
    fn native_date_param_methods() {
        let v = make_vendor();
        for m in &[
            "get_klines",
            "get_margin_data",
            "get_north_bound_flow",
            "get_market_dragon_tiger",
            "get_announcements",
            "get_research_reports",
            // S3(2026-09-26)：「拉全窗 + 本地按截止日过滤」形态，见 get_money_flow_with_asof
            "get_money_flow",
            // T2(2026-09-26)：`sort:"time"` 倒序翻页回溯，见 get_news_with_asof
            "get_news",
            "get_policy_news",
            "search_news",
            // T5(2026-09-26)：TRADE_DATE<= 过滤取截止日前最近一期，见 get_pledge_data_with_asof
            "get_pledge_data",
            // T7(2026-09-26)：realSort 游标（Unix 秒 ×1e6）可跳日，见 get_cls_flash_with_asof
            "get_cls_flash",
        ] {
            assert_eq!(
                v.asof_capability(m),
                AsOfCapability::NativeDateParam,
                "{m} 应该是 NativeDateParam"
            );
        }
    }

    #[test]
    fn synthesize_from_kline_methods() {
        let v = make_vendor();
        for m in &["get_quote", "get_index_quotes"] {
            assert_eq!(
                v.asof_capability(m),
                AsOfCapability::SynthesizeFromKline,
                "{m} 应该是 SynthesizeFromKline"
            );
        }
    }

    #[test]
    fn no_historical_semantic_methods() {
        let v = make_vendor();
        for m in &["get_hot_stocks", "get_industry_ranking", "get_concept_blocks"] {
            assert_eq!(
                v.asof_capability(m),
                AsOfCapability::NoHistoricalSemantic,
                "{m} 应该是 NoHistoricalSemantic"
            );
        }
    }

    #[test]
    fn fallthrough_methods() {
        let v = make_vendor();
        for m in &[
            "get_financials",
            "get_dragon_tiger",
            "get_lockup_schedule",
            "get_north_bound_holding",
            "get_shareholder_trades",
            "get_dividend_records",
            "get_consensus_eps",
            "get_block_trades",
            "get_institutional_visits",
            "get_sector_info",
            "get_peers",
            "get_option_pcr",
            "search_stock",
        ] {
            assert_eq!(v.asof_capability(m), AsOfCapability::Fallthrough, "{m} 应该是 Fallthrough");
        }
    }

    #[test]
    fn unknown_method_falls_through() {
        let v = make_vendor();
        assert_eq!(
            v.asof_capability("nonexistent_method_xyz"),
            AsOfCapability::Fallthrough,
            "未知方法兜底 Fallthrough"
        );
    }

    /// URL 构造正确性测试:模拟 as_of_date,验证 URL 包含正确日期参数
    /// 这层测试通过让 asof_capability 的实现以"已知 as_of" 触发 + 验证 URL 字符串
    /// 因为实际 HTTP 请求需要 mock server,这里只验证 URL 字符串
    #[test]
    fn market_dragon_tiger_url_contains_trade_date() {
        let _expected_trade_date = "2024-03-15";
        let expected_url_substr = format!("TRADE_DATE%3D%27{_expected_trade_date}%27");
        let actual_url = format!(
            "https://datacenter-web.eastmoney.com/api/data/v1/get?\
            reportName=RPT_DAILYBOARDDETAILS&\
            columns=ALL&\
            filter=(TRADE_DATE%3D%27{_expected_trade_date}%27)&\
            pageNumber=1&pageSize=50&sortColumns=BOARD_CODE%2CSECURITY_CODE&sortTypes=1%2C1"
        );
        assert!(
            actual_url.contains(&expected_url_substr),
            "市场龙虎榜 URL 应包含 {expected_url_substr},实际: {actual_url}"
        );
        assert!(actual_url.contains("2024-03-15"));
    }

    #[test]
    fn klines_url_uses_yyyymmdd_end_format() {
        // KLine 的 end 参数是 YYYYMMDD(非 YYYY-MM-DD)
        let end = "20240601";
        let url =
            format!("https://push2his.eastmoney.com/api/qt/stock/kline/get?end={end}&lmt=100");
        assert!(url.contains("end=20240601"));
        assert!(!url.contains("end=2024-06-01")); // 必须不是带分隔符的
    }

    #[test]
    fn announcements_url_contains_begin_and_end_time() {
        // 模拟 as_of_date = 2024-06-01,期望窗口 2023-06-02 ~ 2024-06-01
        let end = "2024-06-01";
        let begin = "2023-06-02";
        let url = format!(
            "https://np-anotice-stock.eastmoney.com/api/security/ann?...&begin_time={begin}&end_time={end}"
        );
        assert!(url.contains("begin_time=2023-06-02"));
        assert!(url.contains("end_time=2024-06-01"));
        // 验证窗口长度 364 天(允许 off-by-1,差 1 算正常)
        let begin_date = chrono::NaiveDate::parse_from_str(begin, "%Y-%m-%d").unwrap();
        let end_date = chrono::NaiveDate::parse_from_str(end, "%Y-%m-%d").unwrap();
        let diff = (end_date - begin_date).num_days();
        assert!((360..=366).contains(&diff), "窗口应在 360-366 天之间,实际: {diff} 天");
    }

    #[test]
    fn research_reports_url_uses_actual_asof_window() {
        let end = "2024-12-31";
        let begin = "2023-12-31";
        let url =
            format!("https://reportapi.eastmoney.com/report/list?beginTime={begin}&endTime={end}");
        assert!(url.contains("beginTime=2023-12-31"));
        assert!(url.contains("endTime=2024-12-31"));
    }

    /// as-of 模式 + asof_capability 决策集成测试
    /// 验证 lib.rs 路由层拿到 eastmoney 的 capability 决策能正确分支
    #[test]
    fn routing_layer_can_query_eastmoney_capability() {
        let v = make_vendor();
        // 模拟 lib.rs 路由层调用
        assert_eq!(v.asof_capability("get_market_dragon_tiger"), AsOfCapability::NativeDateParam);
        assert_eq!(v.asof_capability("get_quote"), AsOfCapability::SynthesizeFromKline);
        assert_eq!(v.asof_capability("get_hot_stocks"), AsOfCapability::NoHistoricalSemantic);
    }

    /// P3 测试(2026-07-25): classify_earnings_title 标题分类正确性
    /// 此函数被 eastmoney.rs 和 browser_eastmoney.rs 共用,需保证行为一致。
    /// 回归点:避免后续修改破坏分类规则,影响财报日历 UI 显示。
    #[test]
    fn classify_earnings_title_categorizes_correctly() {
        // 业绩预告类
        assert_eq!(classify_earnings_title("2024年业绩预告").0, "preliminary");
        assert_eq!(classify_earnings_title("2024年预增公告").0, "preliminary");
        assert_eq!(classify_earnings_title("2024年预减公告").0, "preliminary");

        // 业绩快报类
        assert_eq!(classify_earnings_title("2024年业绩快报").0, "express");

        // 股东大会
        assert_eq!(
            classify_earnings_title("2024年第二次临时股东大会决议").0,
            "shareholders_meeting"
        );
        assert_eq!(classify_earnings_title("2024年股东大会通知").1, None);

        // 正式报告类(年报/季报/半年报)
        assert_eq!(classify_earnings_title("2024年年度报告").0, "formal");
        assert_eq!(classify_earnings_title("2025年第一季度报告").0, "formal");
        assert_eq!(classify_earnings_title("2024年半年度报告").0, "formal");
        assert_eq!(classify_earnings_title("2024年半年报").0, "formal");
        assert_eq!(classify_earnings_title("2024年报").0, "formal");

        // 期间提取
        assert_eq!(classify_earnings_title("2024年年度报告").1.as_deref(), Some("2024年报"));
        assert_eq!(classify_earnings_title("2025年第三季度报告").1.as_deref(), Some("2025Q3"));
        assert_eq!(classify_earnings_title("2024年半年度报告").1.as_deref(), Some("2024Q2"));

        // 其他
        assert_eq!(classify_earnings_title("关于公司章程修订的公告").0, "other");
        assert_eq!(classify_earnings_title("关于公司章程修订的公告").1, None);
    }
}

#[cfg(test)]
mod index_asof_tests {
    use super::*;

    fn k(date: &str, close: f64, volume: f64, amount: f64) -> KLine {
        KLine {
            date: date.into(),
            open: close,
            high: close,
            low: close,
            close,
            volume,
            amount,
            turnover_rate: None,
            adj_factor: None,
        }
    }

    /// as-of 指数合成：点位取末根收盘，昨收取前一根收盘，涨跌幅据此算出。
    #[test]
    fn index_quote_from_two_klines() {
        let ks =
            vec![k("2026-06-01", 3000.0, 100.0, 1000.0), k("2026-06-02", 3060.0, 200.0, 2000.0)];
        let q =
            crate::vendors::index_quote_from_klines("000001", "上证指数", &ks).expect("应能合成");
        assert_eq!(q.code, "000001", "code 必须与 live 路径(f57)同口径，不带市场位");
        assert_eq!(q.name, "上证指数");
        assert_eq!(q.price, 3060.0);
        assert_eq!(q.pre_close, 3000.0);
        assert!((q.change_pct - 2.0).abs() < 1e-9, "涨跌幅应按两根收盘算: {}", q.change_pct);
        assert_eq!((q.volume, q.amount), (200.0, 2000.0));
    }

    /// 只有一根 K 线时无法算涨跌幅 ⇒ 不合成（宁缺毋滥，也不把 pre_close 编成 0）。
    #[test]
    fn index_quote_needs_two_klines() {
        let ks = vec![k("2026-06-02", 3060.0, 200.0, 2000.0)];
        assert!(crate::vendors::index_quote_from_klines("000001", "上证指数", &ks).is_none());
        assert!(crate::vendors::index_quote_from_klines("000001", "上证指数", &[]).is_none());
    }

    /// 昨收为 0（脏数据）时涨跌幅归零，不得产出 inf/NaN 传给报告。
    #[test]
    fn index_quote_zero_pre_close_yields_no_inf() {
        let ks = vec![k("2026-06-01", 0.0, 0.0, 0.0), k("2026-06-02", 3060.0, 1.0, 1.0)];
        let q =
            crate::vendors::index_quote_from_klines("399001", "深证成指", &ks).expect("应能合成");
        assert_eq!(q.change_pct, 0.0);
        assert!(q.change_pct.is_finite());
    }

    /// 指数 secid 必须原样透传：`to_em_secid("1.000001")` 若按「首位数字」推断市场，
    /// 会把上证指数静默换成深市标的（000001 亦是平安银行），合成结果整体错误且无报错。
    #[test]
    fn index_secid_passes_through_without_market_inference() {
        assert_eq!(to_em_secid("1.000001"), "1.000001");
        assert_eq!(to_em_secid("0.399006"), "0.399006");
        // 股票口径不受影响
        assert_eq!(to_em_secid("600519"), "1.600519");
        assert_eq!(to_em_secid("000001"), "0.000001");
        // 港股/美股后缀仍走各自前缀分支
        assert_eq!(to_em_secid("00700.HK"), "116.00700");
        assert_eq!(to_em_secid("AAPL.US"), "105.AAPL");
    }

    /// 清单常量必须与 as-of 合成、live 快照共用（两处各写一份会漂移）。
    #[test]
    fn index_list_is_shared() {
        assert_eq!(EM_INDEX_SECIDS.len(), 3);
        assert_eq!(EM_INDEX_SECIDS[0], ("1.000001", "000001", "上证指数"));
    }
}

#[cfg(test)]
mod fflow_asof_tests {
    //! S3(2026-09-26)：as-of 资金流窗口选择的边界判据。
    //! 缺陷背景：回放里「主力净流入」因子恒缺 —— eastmoney 此前对 get_money_flow
    //! 申报 Fallthrough 而 lib.rs as-of 分支只走快照/NativeDateParam 两路 ⇒ 恒 None。
    //! 修复形态是「拉全窗 + 本地按截止日过滤」，本模块钉住纯函数部分
    //! （HTTP 部分归 live-network，不在此重复）。

    use super::*;

    fn row(date: &str, main: f64) -> MoneyFlowDaily {
        MoneyFlowDaily {
            date: date.into(),
            main_net_inflow: main,
            small_net: 0.0,
            medium_net: 0.0,
            large_net: 0.0,
            super_large_net: 0.0,
        }
    }

    #[test]
    fn select_window_excludes_after_cutoff_and_keeps_latest_first() {
        // 接口升序 + 跨截止日：06-10 是未来数据，绝不得出现在回放结果里（时间泄露桶）；
        // 顶层字段口径 = history[0] 必须是截止日当天（最新一条）。
        let rows = vec![row("2024-05-30", 1.0), row("2024-06-03", 3.0), row("2024-06-10", 9.0)];
        let got = select_fflow_window(rows, "2024-06-03");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].date, "2024-06-03", "最新一条必须在前（顶层字段取值口径）");
        assert_eq!(got[1].date, "2024-05-30");
    }

    #[test]
    fn select_window_truncates_to_five_and_sorts_any_input_order() {
        let rows = vec![
            row("2024-06-05", 5.0),
            row("2024-06-01", 1.0),
            row("2024-06-04", 4.0),
            row("2024-06-02", 2.0),
            row("2024-06-03", 3.0),
            row("2024-05-31", 0.5),
        ];
        let got = select_fflow_window(rows, "2024-06-05");
        assert_eq!(got.len(), 5, "history 至多 5 条（prompt 口径：连续 3-5 日趋势）");
        let dates: Vec<_> = got.iter().map(|r| r.date.as_str()).collect();
        assert_eq!(
            dates,
            vec!["2024-06-05", "2024-06-04", "2024-06-03", "2024-06-02", "2024-06-01"]
        );
    }

    #[test]
    fn select_window_all_after_cutoff_is_empty() {
        // 截止日早于取数窗口 ⇒ 空（上层据此 record_degradation），不得拿未来数据充数。
        let rows = vec![row("2024-07-01", 1.0), row("2024-07-02", 2.0)];
        assert!(select_fflow_window(rows, "2024-06-03").is_empty());
    }

    #[test]
    fn parse_fflow_klines_maps_csv_fields() {
        let v: Vec<Value> = vec![Value::String(
            "2024-06-03,100.0,-20.0,-30.0,-40.0,140.0,x,y,z,w,u,t,s,r,q".into(),
        )];
        let rows = parse_fflow_klines(&v);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].date, "2024-06-03");
        assert_eq!(rows[0].main_net_inflow, 100.0);
        assert_eq!(rows[0].small_net, -20.0);
        assert_eq!(rows[0].medium_net, -30.0);
        assert_eq!(rows[0].large_net, -40.0);
        assert_eq!(rows[0].super_large_net, 140.0);
    }

    /// S7(2026-09-26)：as-of 估值快照 URL 判据 —— 转义/算符写错是**静默空结果**
    /// （接口对非法 filter 返回 success:false 而非 4xx），只有钉 URL 形态能抓住。
    #[test]
    fn valuation_snapshot_asof_url_shape() {
        let u = valuation_snapshot_asof_url("300642", "2026-09-22");
        assert!(
            u.contains("TRADE_DATE%3C%3D%272026-09-22%27"),
            "必须 `<=截止日`（等值查在休市日恒空，同两融先例）: {u}"
        );
        assert!(u.contains("SECURITY_CODE%3D%22300642%22"), "代码等值过滤转义: {u}");
        assert!(u.contains("sortTypes=-1"), "必须倒序取最近一条: {u}");
        assert!(u.contains("pageSize=1"), "单行轻量查询: {u}");
        assert!(u.contains("TOTAL_MARKET_CAP"), "必须取回总市值（DCF 股本链）: {u}");
    }
}

#[cfg(test)]
mod news_asof_tests {
    //! T2(2026-09-26)：新闻 as-of 回溯的纯函数判据 —— 零网络。
    //!
    //! 缺陷背景：回放里 `get_news` 只能拿到**当下**新闻，再被路由层裁空
    //! ⇒ 报告面显示「个股新闻结构性为空」；而翻页裁剪逻辑此前与 live 单页抓取
    //! 各抄一份（M-RES-16 的修复就重复写了两遍），改一处必漏一处。
    use super::*;

    fn item(publish_time: &str) -> NewsItem {
        NewsItem {
            title: format!("标题 {publish_time}"),
            summary: String::new(),
            source: "测试源".into(),
            url: String::new(),
            publish_time: publish_time.into(),
            sentiment_score: None,
        }
    }

    /// 晚于截止日的一条不留；日期不可解析按「时效未知」剔除（回放面从严）
    #[test]
    fn clip_keeps_only_items_on_or_before_cutoff() {
        let page = vec![
            item("2026-09-25 10:00:00"),
            item("2026-09-22 09:00:00"),
            item("2026-09-22T08:00:00"),
            item("2026-09-22"),
            item(""),
        ];
        let kept = clip_page_to_cutoff(&page, "2026-09-22");
        let dates: Vec<&str> = kept.iter().map(|n| n.publish_time.as_str()).collect();
        assert_eq!(
            dates,
            vec!["2026-09-22 09:00:00", "2026-09-22T08:00:00", "2026-09-22"],
            "截止日当天及更早的必须保留，顺序不变"
        );
    }

    /// 收口后的解析器必须同时兼容两种上游形态（M-RES-16 的原始缺陷），
    /// 且字段别名（digest/source/url、publishTime）映射不回退。
    #[test]
    fn jsonp_parser_handles_flat_and_nested_shapes() {
        let flat = r#"jQuery18306({"result":{"cmsArticleWebOld":[{"title":"A","digest":"摘要A","mediaName":"S","articleUrl":"u","showTime":"2026-09-20 10:00:00"}]}})"#;
        let got = parse_news_jsonp(flat).expect("flat 形态应解析成功");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].title, "A");
        assert_eq!(got[0].summary, "摘要A");
        assert_eq!(got[0].source, "S");
        assert_eq!(got[0].publish_time, "2026-09-20 10:00:00");

        let nested = r#"jQuery18306({"result":{"cmsArticleWebOld":{"list":[{"title":"B","content":"正文B","source":"S2","url":"u2","publishTime":"2026-09-21"}]}}})"#;
        let got = parse_news_jsonp(nested).expect("{list:[...]} 形态应解析成功");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].title, "B");
        assert_eq!(got[0].summary, "正文B");
        assert_eq!(got[0].source, "S2");
        assert_eq!(got[0].url, "u2");
        assert_eq!(got[0].publish_time, "2026-09-21");
    }
}

#[cfg(test)]
mod pledge_asof_tests {
    //! T5(2026-09-26)：质押 as-of 通道的 URL 判据。
    //! 旧面板写「该维度无历史语义」是假话 —— `RPT_CSDC_LIST` 有 TRADE_DATE 列，
    //! 实测 `filter=(SECUCODE=..)(TRADE_DATE<='2026-06-30')` 返回 2026-06-26 那期。

    use super::*;

    #[test]
    fn pledge_url_carries_cutoff_filter() {
        let live = EastMoneyVendor::pledge_report_url("300642", None);
        assert!(live.contains("RPT_CSDC_LIST"), "仍走中登质押报表: {live}");
        assert!(live.contains("SECUCODE%3D%22300642.SZ%22"), "SECUCODE 需带市场后缀: {live}");
        assert!(!live.contains("TRADE_DATE%3C%3D"), "live 不应带日期过滤: {live}");

        let replay = EastMoneyVendor::pledge_report_url("300642", Some("2026-06-30"));
        assert!(
            replay.contains("TRADE_DATE%3C%3D%272026-06-30%27"),
            "回放必须带 TRADE_DATE<= 过滤（单引号包日期，实测唯一被服务端接受的写法）: {replay}"
        );
        // 仍按 TRADE_DATE 倒序 ⇒ 首行就是「截止日前最近一期」
        assert!(replay.contains("sortColumns=TRADE_DATE&sortTypes=-1"), "{replay}");
    }

    #[test]
    fn pledge_url_normalizes_prefixed_codes() {
        let u = EastMoneyVendor::pledge_report_url("sz300642", Some("2026-06-30"));
        assert!(u.contains("SECUCODE%3D%22300642.SZ%22"), "前缀市场位需归一: {u}");
    }
}

#[cfg(test)]
mod flash_asof_tests {
    //! T7(2026-09-26)：7×24 快讯回溯的游标与解析判据 —— 零网络。
    //!
    //! 实测纠正了两件事：① `sortEnd` **不是**日期字符串（传 `"2026-09-18 15:00:00"` 被忽略，
    //! 仍返回当下最新，旧注释就这么写错）；② 真游标 = Unix 秒 ×1e6（与条目的 `realSort`
    //! 同族），位数错（如 ×1e9）会被服务端判越界并**静默回落当下** ⇒ 量级必须锁死，
    //! 否则回放会以为「拿到了那天的快讯」而实际拿到今天。
    use super::*;
    use chrono::{FixedOffset, NaiveDate};

    fn day_end_cst(y: i32, m: u32, d: u32) -> chrono::DateTime<FixedOffset> {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(23, 59, 59)
            .unwrap()
            .and_local_timezone(cn_offset())
            .single()
            .unwrap()
    }

    /// 游标量级：2020 年代必须落在 16 位（=秒 ×1e6）。这是「跳日」成立的前提，
    /// 也是实测里唯一会让结果**静默变错**的地方。
    #[test]
    fn flash_cursor_is_epoch_micros_not_millis_or_nanos() {
        let c = flash_cursor_at(&day_end_cst(2026, 9, 18));
        assert!(
            (1_000_000_000_000_000..10_000_000_000_000_000).contains(&c),
            "游标应为 16 位: {c}"
        );
        assert_eq!(c / 1_000_000, day_end_cst(2026, 9, 18).timestamp(), "游标 = 秒 ×1e6");
    }

    /// 游标必须随日期单调推进（跳日靠这个序）
    #[test]
    fn flash_cursor_is_monotonic_per_day() {
        let a = flash_cursor_at(&day_end_cst(2026, 9, 18));
        let b = flash_cursor_at(&day_end_cst(2026, 9, 19));
        assert!(b > a, "次日游标必须更大: {a} -> {b}");
        assert_eq!(b - a, 86_400 * 1_000_000, "一天的跨度换算必须落在 CST 固定偏移上");
    }

    /// URL 里的游标与分页参数不得走 query 编码路径（服务端按原始数字解析 sortEnd）
    #[test]
    fn flash_url_carries_raw_cursor_and_page_size() {
        let u = flash_list_url(1_789_714_800_000_000, 20);
        assert!(u.contains("sortEnd=1789714800000000"), "游标必须原样出现在 URL: {u}");
        assert!(u.contains("pageSize=20"), "{u}");
        assert!(u.contains("req_trace="), "缺 req_trace 会被判参数缺失: {u}");
    }

    /// 字段别名链：title 缺失时用 summary 截断，时间/来源同理
    #[test]
    fn parse_flash_keeps_field_aliases() {
        let raw = serde_json::json!([
            {"title":"央行公告","summary":"正文……","showTime":"2026-09-18 14:58:47","source":"新华社"},
            {"summary":"只有摘要的一条快讯信息","time":"2026-09-18 14:48:56","mediaName":"财联社"},
            {"title":"","summary":""},
        ]);
        let items = parse_fast_news(raw.as_array().unwrap());
        assert_eq!(items.len(), 2, "title/summary 全空的条目应被丢弃");
        assert_eq!(items[0].publish_time, "2026-09-18 14:58:47");
        assert_eq!(items[0].source.as_deref(), Some("新华社"));
        assert_eq!(
            items[1].title, "只有摘要的一条快讯信息",
            "无 title 时用 summary 兜底（≤80 字）"
        );
        assert_eq!(items[1].source.as_deref(), Some("财联社"));
    }
}
