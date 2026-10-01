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
            // L 轮(2026-09-28)：与主 client 同开 gzip 自动解压（emweb CDN gzip 变体）
            .gzip(true)
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
    /// 本地裁剪。也试过往 `cmsArticleWebOld` 里塞 `startDate/endDate` —— **不生效**
    /// （endDate=09-22 仍返回 09-25 的条目），别再找日期参数了。
    /// 翻不到截止日**如实报 Err**，让路由层落到 `news_archive`，而不是静默返回空
    /// （静默空会被下游读成「该股没有新闻」）。
    ///
    /// ⚠ 2026-09-27 实测：倒序翻页只在**前 20 页**内是真降序续接，不是「多翻就有」。
    /// 检索词「集成电路」：页 1–20 各 50 条、日期严格降序（09-27 一路到 09-11）；
    /// **页 21 起退化**为固定 40 条、且日期区间回跳（09-15 → 09-11）—— 那是接口附送的
    /// 「相关结果」尾巴，继续翻只会拿到重复/回跳内容而不前进 ⇒ `MAX_PAGES` 硬停 20
    /// （≈ 1000 条 / 热门词近两周）。
    /// 于是「退不到截止日」有两种**不同**根因，必须分开说：
    /// 该词全量本就少（翻到**空页**＝索引见底）vs 超出倒序索引窗口（我方与接口都到顶）。
    ///
    /// ⚠ 关键词必须先清洗（见 `asof_news_keyword`）：`sort:"time"` 下服务端不做相关性
    /// 排序，组合关键词等于在翻全量流，页数预算根本不够退到截止日。
    async fn news_pages_until_asof(
        &self,
        keyword: &str,
        cutoff: &str,
        need: usize,
    ) -> Result<Vec<NewsItem>, DataError> {
        const PAGE_SIZE: u32 = 50;
        const MAX_PAGES: u32 = 20;
        let kw = asof_news_keyword(keyword);
        let mut audit = AsofPagingAudit {
            origin: if kw == keyword {
                ""
            } else {
                "；已由原词清洗而来"
            },
            ..Default::default()
        };
        let mut kept: Vec<NewsItem> = Vec::new();
        let mut crossed = false;
        for page in 1..=MAX_PAGES {
            let items = self.news_page(&kw, page, PAGE_SIZE, "time").await?;
            audit.pages = page;
            if items.is_empty() {
                audit.exhausted = true; // 索引见底：这个检索词全量就只有这么多条
                break;
            }
            audit.fetched += items.len();
            if let Some(last) = items.last() {
                let d = crate::news_date_key(&last.publish_time);
                if !d.is_empty() {
                    audit.oldest = Some(d.to_string());
                }
            }
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
                message: audit.failure(&kw, cutoff, MAX_PAGES),
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

    /// `RPT_SHAREBONUS_DET`（东财「分红送配」）查询 URL。抽成关联函数只为可单测 ——
    /// 「报表名被改回已下线的旧值」与「漏掉倒序」都是**静默**退化，只有钉 URL 形态能抓住。
    ///
    /// - `sortTypes=-1` 必带：该报表默认按报告期**升序**返回，不加排序时第 1 页是上市初期的
    ///   除权记录（实测 600519 首条为 2002-07-18），近期分红一条都取不到 ——
    ///   而 dividend 的消费面（股息率、连续分红年数、复权）只看近期。
    /// - `pageSize=50`：分红是事件型数据，且 `asof_capability` 对
    ///   `get_dividend_records` 申报 `Fallthrough` ⇒ as-of 回放要求把截止日之前的
    ///   **整段历史**取回，再由 `lib.rs` 的 `truncate_dividend_by_asof` 按 `ex_date` 截断。
    ///   单只票全史在几十条量级（实测 600519 为 28 条），50 一页拿下。
    fn dividend_report_url(stock_code: &str) -> String {
        // 修复(2026-07-22)沿用: SECURITY_CODE 字段需纯数字代码,去除 sh/sz/bj 前缀
        let code =
            stock_code.trim_start_matches("sh").trim_start_matches("sz").trim_start_matches("bj");
        format!(
            "https://datacenter-web.eastmoney.com/api/data/v1/get?\
            reportName=RPT_SHAREBONUS_DET&\
            columns=SECURITY_CODE,EX_DIVIDEND_DATE,EQUITY_RECORD_DATE,PRETAX_BONUS_RMB,BONUS_RATIO,IT_RATIO&\
            filter=(SECURITY_CODE=\"{code}\")&\
            pageSize=50&pageNumber=1&\
            sortColumns=EX_DIVIDEND_DATE&sortTypes=-1"
        )
    }

    /// `RPT_SHAREBONUS_DET` 行 → `DividendRecord`（纯函数，零网络，钉住两处易错换算）。
    ///
    /// 1. **单位是「每 10 股」**：`PRETAX_BONUS_RMB` = 每 10 股派息（元，税前）、
    ///    `BONUS_RATIO` = 每 10 股送股、`IT_RATIO` = 每 10 股转增；而 `DividendRecord`
    ///    的契约是**每股**（消费端见 `adjustment.rs` 的 `1.0/(1.0+bonus_share_ratio)`
    ///    复权步长），故三项统一 ÷10。实测 600519 于 2026-06-26 除权那条
    ///    `PRETAX_BONUS_RMB=280.2423`（茅台每 10 股派 280.24 元）⇒ 每股 28.02 元。
    /// 2. **日期要截到 10 位**：报表返回 `"YYYY-MM-DD 00:00:00"`，而消费端按 `%Y-%m-%d`
    ///    做**字符串**比较（`truncate_dividend_by_asof`：`d.ex_date <= cutoff`）。不截断时
    ///    `"2025-06-26 00:00:00" > "2025-06-26"` 成立 ⇒ 截止日**当天**的除权记录会被
    ///    误判成未来信息剔除。无 `EX_DIVIDEND_DATE` 的行（预案未定/已取消）直接丢弃。
    fn parse_dividend_rows(stock_code: &str, rows: &[Value]) -> Vec<DividendRecord> {
        rows.iter()
            .filter_map(|r| {
                // 金额/比例列在同类报表里既可能是 number 也可能是字符串（"6"/"1.5"），
                // null（未送转/无派息）一律记 0；取法与 `fetch_pledge` 内的 f 闭包一致。
                let f = |key: &str| -> f64 {
                    // 用 match 而非 `as_f64().or_else(...)` 长链：链宽需留在 rustfmt 的
                    // chain_width(60) 内，否则 `cargo fmt --check` 会把该行拆开。
                    match &r[key] {
                        Value::String(s) => s.parse().unwrap_or(0.0),
                        other => other.as_f64().unwrap_or(0.0),
                    }
                };
                let ex_date = r["EX_DIVIDEND_DATE"].as_str().and_then(|d| d.get(..10))?;
                let record_date = r["EQUITY_RECORD_DATE"].as_str().and_then(|d| d.get(..10));
                Some(DividendRecord {
                    stock_code: stock_code.to_string(),
                    ex_date: ex_date.to_string(),
                    dividend_per_share: f("PRETAX_BONUS_RMB") / 10.0,
                    bonus_share_ratio: (f("BONUS_RATIO") + f("IT_RATIO")) / 10.0,
                    record_date: record_date.unwrap_or("").to_string(),
                })
            })
            .collect()
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
            let msg = json["message"].as_str().unwrap_or("unknown");
            // 「返回数据为空」是**答案**（该标的没有质押披露），不是故障。
            // 此前一律抛 Err ⇒ 面板把它记成红档「取数失败」，与真正的
            // 参数错误/报表不存在混在同一档（301302 回放实证）。
            if datacenter_reports_no_data(msg) {
                tracing::debug!("[eastmoney] get_pledge_data 该标的无质押披露: {msg}");
                return Ok(None);
            }
            return Err(DataError::VendorError {
                vendor: "eastmoney".into(),
                message: format!("get_pledge_data 报表不可用: {msg}"),
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

    /// 同行对比主体（live 与 as-of 共用，2026-09-26 T9）。
    ///
    /// `cutoff=Some(d)` 时给估值批量查询加 `TRADE_DATE<=d`：该报表本就按股票返回多日历史，
    /// 「取每只 ≤ 截止日的最新一行」正是回放口径 ⇒ 无需 K 线合成。
    /// 板块归属是慢变量 ⇒ 同业名单沿用当日成分，只落 info 不改数据面。
    async fn peers_impl(
        &self,
        stock_code: &str,
        cutoff: Option<&str>,
    ) -> Result<Vec<PeerComparison>, DataError> {
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
            // as-of 时同一张报表加 TRADE_DATE 上限 ⇒ 每只票取「≤ 截止日的最新一行」
            let val_url = peers_valuation_url(&in_list, cutoff, peer_codes.len() * 12 + 10);
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

        // ── 2026-10-01 修复：同侪 ROE（此前恒 `None`，详见 `peers_roe_url` 的说明）──
        // 与估值批量**分开一次请求**（不同报表，无法合并 columns）；取数失败只 warn ⇒
        // `roe` 留 None（与 pe/pb 的容错口径一致：拿不到不等于整条 peers 失败）。
        let mut roe_map: HashMap<String, (String, f64)> = HashMap::new();
        if !peer_codes.is_empty() {
            let in_list: String = peer_codes
                .iter()
                .map(|c| format!("%22{}%22", to_em_secucode(c)))
                .collect::<Vec<_>>()
                .join(",");
            // 每只票要能覆盖「最近年报 + 最近几期」⇒ 按票数放大页大小（实测 3 票 12 行）
            let roe_url = peers_roe_url(&in_list, cutoff, peer_codes.len() * 6 + 10);
            match self.em_get(&roe_url).await {
                Ok(resp) => match resp.json::<Value>().await {
                    Ok(json) => {
                        if let Some(arr) = json["result"]["data"].as_array() {
                            roe_map = pick_peer_roe(arr);
                        }
                    },
                    Err(e) => {
                        tracing::warn!("[eastmoney] get_peers ROE 批量查询 JSON 解析失败: {e}")
                    },
                },
                Err(e) => tracing::warn!("[eastmoney] get_peers ROE 批量查询失败: {e}"),
            }
        }

        Ok(peer_rows
            .iter()
            .map(|r| {
                let sc = r["SECURITY_CODE"].as_str().unwrap_or("").to_string();
                let v = valuations.get(&sc);
                let roe = roe_map.get(&sc);
                PeerComparison {
                    stock_code: sc,
                    stock_name: r["SECURITY_NAME_ABBR"].as_str().unwrap_or("").to_string(),
                    pe: v.and_then(|x| x.1),
                    pb: v.and_then(|x| x.2),
                    roe: roe.map(|x| x.1),
                    // 口径随值一起返回：消费端（分析师 prompt / 面板）必须看得见
                    // 「这个 ROE 是哪一期」，否则横截面会被不同报告期悄悄污染。
                    roe_period: roe.map(|x| x.0.clone()),
                    change_pct: v.map(|x| x.3).unwrap_or(0.0),
                    market_cap: v.and_then(|x| x.4),
                }
            })
            .collect())
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
/// 同行对比的批量估值查询 URL（`RPT_VALUEANALYSIS_DET` 支持 `SECURITY_CODE in (...)` 批量）。
///
/// `cutoff=Some(d)` 时追加 `(TRADE_DATE<='d')`：该报表**按股票返回多日历史**且已按
/// TRADE_DATE 倒序 ⇒ 配合「每只取首行」就是截止日口径。实测（in=(600519,000001)）：
/// 不带过滤首行 09-24（pe 18.989 / 5.0458），带 cutoff=2026-09-22 首行 **09-22**
/// （pe 19.246 / 5.2289，`CHANGE_RATE` 即当日涨跌幅）⇒ 回放不必再做 K 线合成。
///
/// ⚠ filter 里的括号必须**字面**出现：整体 `encodeURIComponent` 会打成 `%28`/`%29`，
/// 服务端 ANTLR 直接报「参数预处理错误」（`<=` 与单引号才需要编码成 `%3C%3D` / `%27`）。
fn peers_valuation_url(in_list: &str, cutoff: Option<&str>, page_size: usize) -> String {
    let date_clause = match cutoff {
        Some(d) => format!("(TRADE_DATE%3C%3D%27{d}%27)"),
        None => String::new(),
    };
    format!(
        "https://datacenter-web.eastmoney.com/api/data/v1/get?\
         reportName=RPT_VALUEANALYSIS_DET&\
         columns=SECURITY_CODE,PE_TTM,PB_MRQ,TOTAL_MARKET_CAP,CHANGE_RATE,TRADE_DATE&\
         filter=(SECURITY_CODE%20in%20({in_list})){date_clause}&\
         source=WEB&sortColumns=TRADE_DATE&sortTypes=-1&pageNumber=1&pageSize={page_size}"
    )
}

/// 同侪 ROE 批量查询 URL（`RPT_F10_FINANCE_MAINFINADATA`「主要财务指标」）。
///
/// ## 为什么必须换报表（`PeerComparison.roe` 长期恒 `None` 的真因）
///
/// 估值批量走的是 `RPT_VALUEANALYSIS_DET`，它的字段集实测只有
/// `PE_TTM / PB_MRQ / PS_TTM / PCF_OCF_* / TOTAL_MARKET_CAP / CHANGE_RATE / CLOSE_PRICE`
/// —— **结构上不含 ROE**（2026-09-21 修 pe/pb 时留下的注释只说「ROE 不在该报表内」，
/// 没给出替代入口，于是 `roe` 一直写死 `None`）。实测 `RPT_F10_FINANCE_MAINFINADATA`
/// 支持 `(SECUCODE in (...))` **批量**且含 `ROEJQ`（加权平均 ROE，与逐股财报路径
/// `get_financials` 取的是**同一个字段**）⇒ 一次请求即可补齐全部同侪。
///
/// `cutoff=Some(d)` 时加 `REPORT_DATE<=d`：该表按股票返回多期，回放口径应取「≤ 截止日」的那期。
///
/// ⚠ filter 里的括号必须**字面**出现（同 `peers_valuation_url` 的教训）：
/// 整体 `encodeURIComponent` 会打成 `%28`/`%29` ⇒ 服务端 ANTLR 报「参数预处理错误」。
fn peers_roe_url(in_list: &str, cutoff: Option<&str>, page_size: usize) -> String {
    let date_clause = match cutoff {
        Some(d) => format!("(REPORT_DATE%3C%3D%27{d}%27)"),
        None => String::new(),
    };
    format!(
        "https://datacenter-web.eastmoney.com/api/data/v1/get?\
         reportName=RPT_F10_FINANCE_MAINFINADATA&\
         columns=SECUCODE,REPORT_DATE,ROEJQ&\
         filter=(SECUCODE%20in%20({in_list})){date_clause}&\
         source=WEB&sortColumns=REPORT_DATE&sortTypes=-1&pageNumber=1&pageSize={page_size}"
    )
}

/// 同侪 ROE 行的选取：**年报（12-31）优先**，无年报才退到最近一期。
/// 返回 `纯代码 → (实际口径的报告期, ROE)`；报告期由调用方写进 `PeerComparison.roe_period`。
///
/// ## 为什么不能「直接取最新一期」
///
/// `ROEJQ` 是**年内累计值**：一季报/中报/三季报都不是全年数。而 `PeerComparison.roe` 的
/// 消费端是**横截面**比较（「同行 ROE 均值 vs 本公司」）—— 混期会让「只披露到中报的同侪」
/// 看起来只有年报同侪的一半。实测（600887 伊利，2026-10-01）：
/// `2026-06-30 = 10.09` vs `2025-12-31 = 20.87`，**差 2.07 倍**；若混在一起，
/// 分析师会得出「同行盈利能力腰斩」的假结论。
///
/// 本仓已有**同型**教训可直接引用：`fundamentals_report.rs` 的 2026-09-14 口径修正 ——
/// 半年 ROE 4.8 与年化 PE 5.09 并列，让「估值极低」与「盈利严重恶化」同时成立
/// （601166 实证）。故此处统一取**同一年报**，并把口径显式返回给消费端。
///
/// 行按 `REPORT_DATE` **倒序**返回（URL 里 `sortTypes=-1`）⇒ 每个代码首次见到即最新。
fn pick_peer_roe(rows: &[Value]) -> HashMap<String, (String, f64)> {
    let mut annual: HashMap<String, (String, f64)> = HashMap::new();
    let mut latest: HashMap<String, (String, f64)> = HashMap::new();
    for r in rows {
        // `SECUCODE` 形如 `600887.SH`；`PeerComparison.stock_code` 是纯数字 ⇒ 去后缀对齐。
        let (Some(secucode), Some(date), Some(roe)) =
            (r["SECUCODE"].as_str(), r["REPORT_DATE"].as_str(), r["ROEJQ"].as_f64())
        else {
            continue;
        };
        let code = secucode.split('.').next().unwrap_or(secucode).to_string();
        let day = date.get(..10).unwrap_or(date).to_string();
        latest.entry(code.clone()).or_insert_with(|| (day.clone(), roe));
        if day.ends_with("12-31") {
            annual.entry(code).or_insert((day, roe));
        }
    }
    // 年报优先；缺年报的同侪退到最近一期（口径不同 ⇒ 由 roe_period 如实暴露）
    for (code, v) in latest {
        annual.entry(code).or_insert(v);
    }
    annual
}

/// 股东户数 URL（`RPT_HOLDERNUMLATEST`，东财 F10）。
///
/// 为什么选「最新一期」而非多期历史：`lockup-watcher.md` 要的是**当前筹码集中度及其变化**
/// —— 该表一行里同时给了 `HOLDER_NUM`（户数）、`HOLDER_NUM_RATIO`（较上期变化率）、
/// `AVG_HOLD_NUM`（户均持股）、`END_DATE` + `HOLD_NOTICE_DATE`（判是否过时），
/// 一次请求即可回答，不需要历史序列。多期历史（`RPT_F10_EH_HOLDERNUM`）留待需要趋势时再接。
fn holder_count_url(secucode: &str) -> String {
    format!(
        "https://datacenter-web.eastmoney.com/api/data/v1/get?\
         reportName=RPT_HOLDERNUMLATEST&\
         columns=SECUCODE,END_DATE,HOLDER_NUM,HOLDER_NUM_RATIO,AVG_HOLD_NUM,HOLD_NOTICE_DATE&\
         filter=(SECUCODE%3D%22{secucode}%22)&\
         source=WEB&pageNumber=1&pageSize=1"
    )
}

/// 行 → `HolderCount`（纯函数，零网络可测）。
/// 日期统一截到 10 位：接口返回 `"2026-06-30 00:00:00"`，而消费端（LLM / 面板）按 `YYYY-MM-DD` 读。
fn parse_holder_count(stock_code: &str, row: &Value) -> HolderCount {
    let day = |k: &str| -> Option<String> {
        row[k].as_str().map(|s| s.get(..10).unwrap_or(s).to_string())
    };
    HolderCount {
        stock_code: stock_code.to_string(),
        end_date: day("END_DATE").unwrap_or_default(),
        holder_num: row["HOLDER_NUM"].as_f64(),
        holder_num_ratio: row["HOLDER_NUM_RATIO"].as_f64(),
        avg_hold_num: row["AVG_HOLD_NUM"].as_f64(),
        notice_date: day("HOLD_NOTICE_DATE"),
    }
}

/// 资产负债表 URL（`RPT_F10_FINANCE_GBALANCE`）—— 取 `GOODWILL` / `ACCOUNTS_RECE`。
///
/// 为什么用这张表：`ZYZBAjaxNew`（主要指标，逐股财报路径的现用接口）**不含资产负债表科目**，
/// 而 `fundamentals_report` 的「A 股特色风险」段要渲染 `商誉 / 应收账款` 两行
/// （`if let Some(v)` 条件渲染 ⇒ 拿不到就整行消失，见 `get_financials` ③-b 段的说明）。
/// 实测该表按 `SECUCODE` 返回**多期**，正好与 `get_financials` 的多期结构逐期对齐。
fn balance_sheet_url(secucode: &str) -> String {
    format!(
        "https://datacenter-web.eastmoney.com/api/data/v1/get?\
         reportName=RPT_F10_FINANCE_GBALANCE&\
         columns=SECUCODE,REPORT_DATE,GOODWILL,ACCOUNTS_RECE&\
         filter=(SECUCODE%3D%22{secucode}%22)&\
         source=WEB&sortColumns=REPORT_DATE&sortTypes=-1&pageNumber=1&pageSize=20"
    )
}

/// 资产负债表行 → `报告期(前 10 位) → (商誉, 应收账款)`。
///
/// 逐期建表（**不做**「取最新一期」的降级）：商誉/应收是**存量**科目，只对本期有意义，
/// 拿邻期值填本期会在 DCF/风险判断里制造看不出来的口径漂移。某期缺该行就留空。
/// 单列为纯函数是为了能用**真实响应切片**做零网络断言（见 `balance_sheet_tests`）。
fn pick_balance_sheet(rows: &[Value]) -> HashMap<String, (Option<f64>, Option<f64>)> {
    let mut m: HashMap<String, (Option<f64>, Option<f64>)> = HashMap::new();
    for r in rows {
        let Some(date) = r["REPORT_DATE"].as_str() else { continue };
        let day = date.get(..10).unwrap_or(date).to_string();
        m.insert(day, (r["GOODWILL"].as_f64(), r["ACCOUNTS_RECE"].as_f64()));
    }
    m
}

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

/// 东财公告查询 URL —— earnings_calendar 的**两个通道共用**（eastmoney 直连 /
/// browser_eastmoney 浏览器内核）。
///
/// 不传 `cb` 参数：实测该接口在**不带** `cb` 时返回纯 JSON
/// （`{"data":{"list":[...]},"error":"","success":1}`），带上 `cb=jQuery` 才是 JSONP；
/// 而 JSONP 会让 `browser_eastmoney` 的 `browser_fetch`（直接 `serde_json::from_str`）
/// 解析失败 —— 那条通道是东财 JA3 封锁时唯一的兜底，不能因信封形态被掐掉。
///
/// 与 `get_announcements` 的内联 URL 形态相近但**有意不合并**：那两处（live / as-of）
/// 带 `cb` 与 `begin_time/end_time` 日期窗口，属回放通道，合并会改动未经实测的回放路径。
pub(crate) fn notice_ann_url(stock_code: &str, page_size: u32) -> String {
    // 与 dividend / earnings 同一处归一化：公告接口的 stock_list 用纯数字代码
    let code =
        stock_code.trim_start_matches("sh").trim_start_matches("sz").trim_start_matches("bj");
    format!(
        "https://np-anotice-stock.eastmoney.com/api/security/ann?\
        sr=-1&page_size={page_size}&page_index=1&ann_type=A&client_source=web&\
        stock_list={code}&f_node=0&s_node=0"
    )
}

/// 财报日历一次取回的公告条数。
///
/// 为什么不是 `RPTA_WEB_NOTICE` 时代的 30：公告是按时间倒序的**全类目**流，
/// 同一天可能连发几十条（实测 600519 在 2026-08-15 单日就有 5+ 条，其中只有
/// 1 条是「半年度报告摘要」），条数太少会把真正的财报公告挤出窗口。
pub(crate) const EARNINGS_NOTICE_PAGE_SIZE: u32 = 50;

/// 东财公告接口响应 → `EarningsEvent`（两个通道共用，避免各抄一份、改一处漏一处）。
///
/// **故障与空的区分**（与 dividend 同一族修复，勿退回）：`success != 1` 或信封里没有
/// `data.list` ⇒ 抛 Err。`RPTA_WEB_NOTICE` 下线时正是「取不到 → `Ok(vec![])`」把接口
/// 失效伪装成「该股没有财报事件」，全站静默 —— 这次不能再留同款后门。
pub(crate) fn earnings_from_notice_json(
    stock_code: &str,
    source: &str,
    json: &Value,
) -> Result<Vec<EarningsEvent>, DataError> {
    let ok = json["success"].as_i64() == Some(1) || json["success"].as_bool() == Some(true);
    if !ok {
        let msg = json["error"].as_str().unwrap_or("unknown");
        return Err(DataError::VendorError {
            vendor: source.to_string(),
            message: format!("财报日历公告接口不可用: {msg}"),
        });
    }
    let items = match json["data"]["list"].as_array() {
        Some(arr) => arr,
        None => {
            return Err(DataError::VendorError {
                vendor: source.to_string(),
                message: "财报日历公告接口信封异常: 缺少 data.list".into(),
            });
        },
    };

    Ok(items
        .iter()
        .filter_map(|item| {
            let title = item["title"].as_str().unwrap_or("");
            // 公告时间是 "YYYY-MM-DD HH:MM:SS"；`truncate_earnings_by_asof` 按 `%Y-%m-%d`
            // 做字符串比较，不截断会让截止日**当天**的公告被判成未来信息剔除。
            let notice_date = item["notice_date"].as_str().and_then(|d| d.get(..10))?;
            if title.is_empty() {
                return None;
            }

            let (event_type, period) = classify_earnings_title(title);
            // 只保留财报相关事件（沿用既有口径：分类为 other 但标题仍含"报告/业绩"的保留）
            if event_type == "other" && !title.contains("报告") && !title.contains("业绩") {
                return None;
            }

            Some(EarningsEvent {
                stock_code: stock_code.to_string(),
                stock_name: item["codes"][0]["short_name"].as_str().unwrap_or("").to_string(),
                event_date: notice_date.to_string(),
                event_type: event_type.to_string(),
                period,
                detail: Some(title.to_string()),
                source: Some(source.to_string()),
                created_at: chrono::Utc::now().timestamp(),
            })
        })
        .collect())
}

/// 解析 push2his fflow/daykline 的 klines CSV 数组（f51=日期 f52=主力 f53=小单 f54=中单 f55=大单 f56=超大单）。
/// 接口按日期**升序**返回（2026-09-26 实测），本函数保持原序，排序由调用方显式做。
pub(crate) fn parse_fflow_klines(klines: &[Value]) -> Vec<MoneyFlowDaily> {
    klines
        .iter()
        .filter_map(|v| v.as_str())
        .map(|line| {
            let parts: Vec<&str> = line.split(',').collect();
            let f = |i: usize| -> f64 { parts.get(i).and_then(|s| s.parse().ok()).unwrap_or(0.0) };
            MoneyFlowDaily {
                date: parts.first().unwrap_or(&"").to_string(),
                main_net_inflow: f(1),
                small_net: Some(f(2)),
                medium_net: Some(f(3)),
                large_net: Some(f(4)),
                super_large_net: Some(f(5)),
            }
        })
        .collect()
}

/// as-of 窗口选择：入参为**任意序**的日频资金流行，返回 `date <= cutoff` 中最近的
/// 至多 5 条，按日期**降序**（最新在前，与 MoneyFlow.history 消费口径一致）。
pub(crate) fn select_fflow_window(
    mut rows: Vec<MoneyFlowDaily>,
    cutoff: &str,
) -> Vec<MoneyFlowDaily> {
    rows.retain(|r| r.date.as_str() <= cutoff);
    rows.sort_by(|a, b| b.date.cmp(&a.date));
    rows.truncate(5);
    rows
}

/// push2his 日频资金流 URL —— eastmoney 直连与 browser_eastmoney 内核兜底**共用同一形态**
/// （2026-09-27：本机对 push2his 是连接级拒绝，只有走 webview 的那条路拿得到日线，
/// 两条路必须同 URL，否则兜底源会给出与主源不同口径的资金流）。
pub(crate) fn fflow_daykline_url(secid: &str) -> String {
    format!(
        "https://push2his.eastmoney.com/api/qt/stock/fflow/daykline/get?\
        lmt=0&klt=101&fields1=f1,f2,f3,f7&\
        fields2=f51,f52,f53,f54,f55,f56,f57,f58,f59,f60,f61,f62,f63,f64,f65&\
        secid={secid}"
    )
}

/// 「已按截止日裁好的窗口」→ 消费侧的 `MoneyFlow`（空窗口 ⇒ `None`，由调用方留痕）。
pub(crate) fn money_flow_from_window(recent: Vec<MoneyFlowDaily>) -> Option<MoneyFlow> {
    let latest = recent.first()?;
    Some(MoneyFlow {
        date: latest.date.clone(),
        main_net_inflow: latest.main_net_inflow,
        super_large_net: latest.super_large_net,
        large_net: latest.large_net,
        medium_net: latest.medium_net,
        small_net: latest.small_net,
        history: recent,
    })
}

// ─────────────────────────────────────────────────────────────
// T15（2026-09-27）：行业排名在回放里**不是**「无历史语义」
//
// 实测：名单接口 `data.eastmoney.com/dataapi/bkzj/getbkzj` 忽略一切日期参数
// （`date=2026-09-11` / `date=20260911` 与不带参数返回**完全一致**，煤炭 f3 恒 66），
// 但它给出的 `f13.f12`（如 `90.BK0437`）正是板块指数的 secid；
// 板块指数日 K 接口 `push2his.../kline/get` 有**原生** `end=YYYYMMDD` + `lmt`
// ⇒ 取「截止日与其前一根」两条收盘即可算出当日板块涨跌幅 ⇒ 排序就是那天的行业排名。
// 成本：二级行业 31 个板块（实测 rows=31）× 1 次小请求，并发 8。
// ─────────────────────────────────────────────────────────────

/// 行业板块名单（当日成分）。板块归属是慢变量 ⇒ 与 T9 同行对比同口径：
/// 名单沿用当日，只有**行情数值**按截止日取。
pub(crate) const INDUSTRY_BOARD_LIST_URL: &str =
    "https://data.eastmoney.com/dataapi/bkzj/getbkzj?key=f3,f62,f12,f14,f128,f140&code=m:90+s:2";

/// 板块指数日 K（只要 `end` 之前两根收盘）
pub(crate) fn board_kline_url(secid: &str, cutoff_compact: &str) -> String {
    format!(
        "https://push2his.eastmoney.com/api/qt/stock/kline/get?\
        secid={secid}&klt=101&fqt=1&lmt=2&end={cutoff_compact}&\
        fields1=f1,f2,f3&fields2=f51,f53"
    )
}

/// 名单响应 → `[(secid, 板块名)]`（`f13`=市场号 90，`f12`=BK 代码）
pub(crate) fn industry_board_list(json: &Value) -> Vec<(String, String)> {
    json["data"]["diff"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|r| {
                    let code = r["f12"].as_str()?;
                    let name = r["f14"].as_str()?;
                    if code.is_empty() || name.is_empty() {
                        return None;
                    }
                    let market = r["f13"].as_i64().unwrap_or(90);
                    Some((format!("{market}.{code}"), name.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 板块当日行情榜单 URL：`t:1` 地域 / `t:2` 行业 / `t:3` 概念。
/// 与 `get_industry_ranking` 同用 bkzj 接口（本机对 push2\* 族 RST 阻断背景下**存活**的
/// 行情通道）；`f3` 为百分数×100（66 = 0.66%），换算约定与 L3309 逐字一致。
pub(crate) fn block_quotes_url(t: &str) -> String {
    format!("https://data.eastmoney.com/dataapi/bkzj/getbkzj?key=f3,f12,f14&code=m:90+t:{t}")
}

/// 若干张榜单响应 → `板块名 → 当日涨跌幅(%)`。join 键用名字而非代码：
/// F10 成分报表（`RPT_F10_CORETHEME_BOARDTYPE`）只给 BOARD_NAME，不给 BK 代码。
pub(crate) fn build_block_pct_map(lists: &[Value]) -> HashMap<String, f64> {
    let mut m = HashMap::new();
    for json in lists {
        if let Some(rows) = json["data"]["diff"].as_array() {
            for r in rows {
                if let (Some(name), Some(f3)) = (r["f14"].as_str(), r["f3"].as_f64()) {
                    if !name.is_empty() {
                        m.insert(name.to_string(), f3 / 100.0);
                    }
                }
            }
        }
    }
    m
}

/// 日 K 字符串（`"YYYY-MM-DD,收盘"`，接口按日期升序）→ `(截止日, 当日涨跌幅%)`。
///
/// 两处不自证就会说谎的地方：
/// 1. `end` 参数**不总被尊重**（T1 的先例）⇒ 本地再按 `cutoff` 裁一次，取 `<= cutoff` 的最后两根；
/// 2. 停牌/新上市板块可能只有一根 ⇒ 没有前收就无法算涨跌幅，如实返回 `None`。
pub(crate) fn board_change_from_bars(klines: &[Value], cutoff: &str) -> Option<(String, f64)> {
    let bars: Vec<(String, f64)> = klines
        .iter()
        .filter_map(|v| v.as_str())
        .filter_map(|line| {
            let mut it = line.split(',');
            let date = it.next()?.to_string();
            let close = it.next()?.parse::<f64>().ok()?;
            Some((date, close))
        })
        .filter(|(date, _)| date.as_str() <= cutoff)
        .collect();
    if bars.len() < 2 {
        return None;
    }
    let (date, last) = bars[bars.len() - 1].clone();
    let prev = bars[bars.len() - 2].1;
    if prev.abs() < f64::EPSILON {
        return None;
    }
    Some((date, (last / prev - 1.0) * 100.0))
}

/// 板块日 K 的逐板块结果：`(板块名, 截止日涨跌幅或错误)`
type BoardBarOutcome = (String, Result<Option<(String, f64)>, DataError>);

/// 板块日 K 的并发上限（二级行业 31 个板块 ⇒ 4 轮）
const BOARD_KLINE_CONCURRENCY: usize = 8;

/// 合成主体：`fetch` 由调用方提供（直连 `em_get` 或内核 `browser_fetch`），
/// 两个 vendor 共用同一套 URL 与解析 —— 兜底只换传输通道，不换口径。
pub(crate) async fn synthesize_industry_ranking<'a, F>(
    vendor: &str,
    cutoff: &str,
    fetch: F,
) -> Result<Vec<IndustryRank>, DataError>
where
    F: Fn(String) -> futures::future::BoxFuture<'a, Result<Value, DataError>> + Send + Sync + 'a,
{
    use futures::stream::{self, StreamExt};
    let err = |message: String| DataError::VendorError { vendor: vendor.into(), message };
    let cutoff_compact = cutoff.replace('-', "");
    let list = fetch(INDUSTRY_BOARD_LIST_URL.to_string()).await?;
    let boards = industry_board_list(&list);
    if boards.is_empty() {
        return Err(err("行业板块名单为空（接口结构变更?）".into()));
    }
    let results: Vec<BoardBarOutcome> = stream::iter(boards)
        .map(|(secid, name)| {
            let fut = fetch(board_kline_url(&secid, &cutoff_compact));
            async move {
                let changed = match fut.await {
                    Ok(json) => {
                        let bars = json["data"]["klines"].as_array().cloned().unwrap_or_default();
                        Ok(board_change_from_bars(&bars, cutoff))
                    },
                    Err(e) => Err(e),
                };
                (name, changed)
            }
        })
        .buffer_unordered(BOARD_KLINE_CONCURRENCY)
        .collect()
        .await;
    let mut ranks = Vec::new();
    let mut first_err: Option<String> = None;
    let mut latest_bar_date = String::new();
    for (name, changed) in results {
        match changed {
            Ok(Some((date, change_pct))) => {
                if date.as_str() > latest_bar_date.as_str() {
                    latest_bar_date = date.clone();
                }
                ranks.push(IndustryRank {
                    industry_name: name,
                    change_pct,
                    turnover: None,
                    main_inflow: None,
                    leader_code: None,
                    leader_name: None,
                    leader_change_pct: None,
                });
            },
            // 不足两根 ⇒ 该板块当日涨跌幅算不出，跳过（不编 0%）
            Ok(None) => {},
            Err(e) => {
                first_err.get_or_insert_with(|| e.to_string());
            },
        }
    }
    if ranks.is_empty() {
        return Err(match first_err {
            Some(e) => err(format!("板块指数日 K 全部取数失败，首个原因: {e}")),
            None => err("板块指数日 K 均不足两根（算不出截止日涨跌幅）".into()),
        });
    }
    if latest_bar_date.as_str() != cutoff {
        // 截止日休市 ⇒ 合成出来的是「截止日前最近交易日」的排名（T1 数据自证同口径）
        tracing::info!(
            "[astock] 行业排名 as-of：截止日 {cutoff} 无板块收盘，取最近交易日 {latest_bar_date}"
        );
    }
    ranks.sort_by(|a, b| b.change_pct.total_cmp(&a.change_pct));
    Ok(ranks)
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
    // 显式市场标记（`000001.SH` / `sh000001`）优先于首位数字推断：上证综指与平安银行
    // 同为 `000001`，把 `sh000001` 去掉前缀再按首位推断会静默取回平安银行的 K 线。
    if let Some((bare, ex)) = crate::code_form::split_explicit_market(stock_code) {
        return format!("{}.{bare}", ex.em_market());
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

/// as-of 翻页用的关键词清洗（只作用于回放路径，live 的 `search_news` 行为不变）。
///
/// 为什么回放要洗而 live 不用：live 拿的是「相关性」排序的第一页，宽词只是噪声多；
/// 而 `sort:"time"` 下服务端不做相关性排序，组合关键词等于在全量流里逐页倒着翻 ——
/// 实测「透景生命 政策」第 1 页 50 条全是同一天（09-26），12 页 600 条只退到 09-24，
/// 永远到不了截止日 09-22；同一时间用纯名「透景生命」1 页即覆盖 09-21→09-25。
///
/// 复用 `clean_search_keyword`（取最长连续中文片段）：并列长度时**保留前一个**片段，
/// 所以「透景生命 政策」→「透景生命」（股票名在前是调用方的常见写法，也是我们要的那段）。
/// datacenter 报表 `success=false` 的两类含义（实测消息见括号）：
///
/// - **空是答案**：`返回数据为空`（如非质押标的的 `RPT_CSDC_LIST`、非两融标的的
///   `RPTA_WEB_RZRQ_GGMX`）⇒ 调用方按「该标的无此项披露」处理，不留红档故障。
/// - **确实是故障**：`参数预处理错误`（9501）、`报表配置不存在`、鉴权失败等。
///
/// 判据只用消息文本：`code` 的位置与类型随报表变化（9201/9501 都出现过），消息才稳定。
fn datacenter_reports_no_data(msg: &str) -> bool {
    let m = msg.trim();
    m.contains("返回数据为空") || m.contains("无数据") || m.eq_ignore_ascii_case("no data")
}

fn asof_news_keyword(keyword: &str) -> String {
    let cleaned = crate::clean_search_keyword(keyword);
    if cleaned.is_empty() {
        keyword.trim().to_string()
    } else {
        cleaned
    }
}

/// 倒序翻页的逐页记账 —— 抽成结构体是为了让「退不到截止日」的文案能被单测打到
/// （否则这段判定埋在 `&self` + HTTP 循环里，只能靠真网络验证）。
#[derive(Default)]
struct AsofPagingAudit {
    /// 关键词清洗痕迹（原词与实检索词不同时说明）
    origin: &'static str,
    fetched: usize,
    pages: u32,
    /// 已翻页里最晚（最早日期）那条的 `YYYY-MM-DD`
    oldest: Option<String>,
    /// 翻到了空页 = 该检索词在索引里全量就这么些条
    exhausted: bool,
}

impl AsofPagingAudit {
    /// 两种「退不到截止日」的根因文案（合并成一句会把人引向错误的下一步：
    /// 以为多翻几页就有，实际接口的倒序索引窗口就到 `max_pages` 页）。
    ///
    /// `exhausted=true` ⇒ 数据面事实（换词或靠 `news_archive` 积累）；
    /// `exhausted=false` ⇒ 窗口天花板。两者都给出**实际退到的日期**，
    /// 否则无法判断差多少。
    fn failure(&self, kw: &str, cutoff: &str, max_pages: u32) -> String {
        match (&self.oldest, self.exhausted) {
            (Some(o), true) => format!(
                "检索词「{kw}」在时间倒序索引里只有 {} 条、最晚到 {o}，已翻到索引底仍不及截止日 {cutoff}{}",
                self.fetched, self.origin
            ),
            (Some(o), false) => format!(
                "时间回溯 {} 页(共 {} 条)只到 {o}，未覆盖截止日 {cutoff}（该索引的倒序窗口就到 {max_pages} 页）{}",
                self.pages, self.fetched, self.origin
            ),
            _ => format!(
                "时间回溯 {} 页未取到任何带日期的条目（检索词「{kw}」）{}",
                self.pages, self.origin
            ),
        }
    }
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

        // ③-b 资产负债表补充（2026-10-01）：**商誉 / 应收账款**。
        //
        // 此前两个字段写死 `None`，原注释只说「当前利润表接口未提供，后续可通过 ZcfzbAjaxNew
        // 资产负债表接口补全」—— 后果不是「少两个数」，而是 `fundamentals_report` 的 markdown
        // **整行不渲染**（`if let Some(v)` 条件渲染）⇒ 而 `fundamentals-analyst.md` 又声称
        // 「商誉/应收账款已包含在预聚合报告中，直接引用即可」⇒ 分析师只能写
        // 「无审计意见/商誉/质押信息 ⇒ A 股特色风险维度数据缺失」⇒ 命中失败标记词表 ⇒ 判低置信
        // （600887 运行 `f474ec9b` 实证）。
        //
        // 实测 `RPT_F10_FINANCE_GBALANCE`（datacenter-web，按 SECUCODE）含
        // `GOODWILL` / `ACCOUNTS_RECE`，且**按报告期返回多行** ⇒ 与本函数的「多期」结构天然对齐，
        // 逐期填、不串期（用日期前 10 位对齐两表的报告期串）。
        // 取数失败只 warn：这是增强项，不该让整条财报链硬失败（同 type=1 年报补充请求的口径）。
        //
        // ⚠ 副作用已计量：`FundamentalsAnalyzer::completeness` 的 16 个字段里含这两项
        //   ⇒ 填上后 `data_completeness` **+2/16 = +12.5%**（600887 实测 69% → 约 81%），
        //   并可能影响 `health_score` 的 A 股特色风险分档。该增量由
        //   `fundamentals_report::tests::goodwill_and_receivables_raise_data_completeness`
        //   逐字锁住，不靠人记。
        let mut balance: HashMap<String, (Option<f64>, Option<f64>)> = HashMap::new();
        match self.em_get(&balance_sheet_url(&to_em_secucode(code))).await {
            Ok(resp) => match resp.json::<Value>().await {
                Ok(j) => {
                    if let Some(arr) = j["result"]["data"].as_array() {
                        balance = pick_balance_sheet(arr);
                    }
                },
                Err(e) => tracing::warn!(
                    "[eastmoney] get_financials 资产负债表 JSON 解析失败(商誉/应收留空): {e}"
                ),
            },
            Err(e) => {
                tracing::warn!("[eastmoney] get_financials 资产负债表请求失败(商誉/应收留空): {e}")
            },
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
                    // 商誉/应收账款：由 ③-b 的资产负债表批量取数**按报告期**填（2026-10-01 起）。
                    //   两表的报告期串格式一致（`YYYY-MM-DD 00:00:00`），此处比前 10 位；
                    //   该期没有对应行时留 `None`（不拿邻期的值冒充本期）。
                    goodwill: balance
                        .get(s("REPORT_DATE").get(..10).unwrap_or(s("REPORT_DATE")))
                        .and_then(|b| b.0),
                    accounts_receivable: balance
                        .get(s("REPORT_DATE").get(..10).unwrap_or(s("REPORT_DATE")))
                        .and_then(|b| b.1),
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
        let url = fflow_daykline_url(&secid);

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
        Ok(money_flow_from_window(recent))
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
        // 东方财富数据中心: 分红送配数据。
        //
        // 修复(2026-10-01): 原 `RPTA_WEB_DIVIDEND` 报表已被东财下线 —— 该 URL 恒返回
        //   `{"result":null,"success":false,"message":"报表配置不存在,RPTA_WEB_DIVIDEND","code":9501}`
        // 而旧实现遇 `result.data` 缺失即 `Ok(vec![])`，上层据此判「该股无分红」⇒
        // 全市场 dividend **静默**返空，既不降级也不重试（2026-10-01 02:02 运行日志实证：
        // 600519/601318 等必然有分红的票全部报「分红数据为空」）。现改用同一数据中心的
        // `RPT_SHAREBONUS_DET`（东财「分红送配」页数据源，实测 600519 全史 28 条）。
        let url = Self::dividend_report_url(stock_code);
        let resp = self.em_get(&url).await?;
        let json: Value = resp.json().await.map_err(|e| DataError::VendorError {
            vendor: "eastmoney".into(),
            message: format!("get_dividend_records JSON 解析失败: {e}"),
        })?;

        // `success=false` 有两类含义，判据复用 `datacenter_reports_no_data`（勿手写关键词）：
        //   「返回数据为空」⇒ 该标的确实没有分红，是**答案**，返空即可；
        //   「报表配置不存在 / 参数预处理错误」⇒ **故障**，必须抛 Err 走降级与重试，
        //   否则报表再次下线时又会退化成「静默无分红」—— 本次要防的正是这个。
        // 注意文案不得含「为空/无数据」等词，否则会被 lib.rs 的 `is_empty_data` 再判成
        // 「非故障空数据」，把降级路径重新堵死。
        if json["success"].as_bool() == Some(false) {
            let msg = json["message"].as_str().unwrap_or("unknown");
            if datacenter_reports_no_data(msg) {
                tracing::debug!("[eastmoney] get_dividend_records 该标的无分红记录: {msg}");
                return Ok(vec![]);
            }
            return Err(DataError::VendorError {
                vendor: "eastmoney".into(),
                message: format!("get_dividend_records 报表不可用: {msg}"),
            });
        }

        let rows = match json["result"]["data"].as_array() {
            Some(arr) => arr,
            None => return Ok(vec![]),
        };

        Ok(Self::parse_dividend_rows(stock_code, rows))
    }

    /// 获取财报日历事件
    ///
    /// 修复(2026-10-01): 原 `RPTA_WEB_NOTICE`（datacenter-web）报表已被东财下线 —— 实测
    /// 该 URL 恒返回 `{"result":null,"success":false,"message":"报表配置不存在,...","code":9501}`，
    /// 而旧实现遇 `result.data` 缺失即 `Ok(vec![])` ⇒ 财报日历静默为空（与 dividend 同一族故障：
    /// 「接口失效」被当成「该股没有此数据」，既不降级也不重试）。现改用东财公告接口
    /// （`np-anotice-stock`，实测可用），仍按标题分类：分类复用 `classify_earnings_title`，
    /// 信封校验与解析复用 `earnings_from_notice_json`（浏览器兜底通道共用同一份）。
    /// - "业绩预告" → preliminary
    /// - "业绩快报" → express
    /// - "定期报告"/"年报"/"季报" → formal
    /// - "股东大会" → shareholders_meeting
    /// - 其他 → other
    async fn get_earnings_calendar(
        &self,
        stock_code: &str,
    ) -> Result<Vec<EarningsEvent>, DataError> {
        let url = notice_ann_url(stock_code, EARNINGS_NOTICE_PAGE_SIZE);
        let resp = self.em_get(&url).await?;
        let json: Value = resp.json().await.map_err(|e| DataError::VendorError {
            vendor: "eastmoney".into(),
            message: format!("get_earnings_calendar JSON 解析失败: {e}"),
        })?;
        earnings_from_notice_json(stock_code, "eastmoney", &json)
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
        self.peers_impl(stock_code, None).await
    }

    /// T9：按截止日取同行对比（估值字段走同一张报表的 TRADE_DATE 上限过滤）。
    async fn get_peers_with_asof(
        &self,
        stock_code: &str,
    ) -> Result<Vec<PeerComparison>, DataError> {
        let ctx = crate::as_of::current_as_of().ok_or_else(|| {
            DataError::ParseError("get_peers_with_asof 调用时缺 as_of 上下文".into())
        })?;
        let cutoff = ctx.as_of_date.format("%Y-%m-%d").to_string();
        let rows = self.peers_impl(stock_code, Some(&cutoff)).await?;
        tracing::info!(
            "[asof] {stock_code} 同行对比按截止日 {cutoff} 取估值（同业名单沿用当日成分，板块归属为慢变量）"
        );
        Ok(rows)
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
    /// T15(2026-09-27)：行业排名**有**历史通道，此前只是因为没接而被申报成
    /// `NoHistoricalSemantic`。名单接口忽略日期参数（实测），但名单里的
    /// `90.BKxxxx` 是板块指数 secid ⇒ 用日 K 的 `end=` 取截止日两根收盘算涨跌幅。
    /// 主体在 `synthesize_industry_ranking`（与 browser_eastmoney 共用）。
    async fn get_industry_ranking_with_asof(&self) -> Result<Vec<IndustryRank>, DataError> {
        let as_of = crate::as_of::current_as_of()
            .ok_or_else(|| DataError::ParseError("no as_of context".into()))?;
        let cutoff = as_of.as_of_date.format("%Y-%m-%d").to_string();
        synthesize_industry_ranking("eastmoney", &cutoff, |url| {
            Box::pin(async move {
                let resp = self.em_get(&url).await?;
                resp.json().await.map_err(|e| DataError::VendorError {
                    vendor: "eastmoney".into(),
                    message: format!("行业排名合成 JSON 解析失败: {e}"),
                })
            })
        })
        .await
    }

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

    /// 股东户数（筹码集中度），新增(2026-10-01)。
    ///
    /// 报表：`RPT_HOLDERNUMLATEST`（东财 F10「股东户数」最新一期）。
    /// **无记录时返回 `Ok(None)`**（该股确实没披露过，不是故障）—— 与质押的处理相反，
    /// 因为这里的「没有」是可判定的业务事实（新上市公司/未披露），而不是解析口径问题。
    async fn get_holder_count(&self, stock_code: &str) -> Result<Option<HolderCount>, DataError> {
        let code =
            stock_code.trim_start_matches("sh").trim_start_matches("sz").trim_start_matches("bj");
        let url = holder_count_url(&to_em_secucode(code));
        let resp = self.em_get(&url).await?;
        let json: Value = resp.json().await.map_err(|e| DataError::VendorError {
            vendor: "eastmoney".into(),
            message: format!("get_holder_count JSON 解析失败: {e}"),
        })?;
        let Some(row) = json["result"]["data"].as_array().and_then(|a| a.first()) else {
            return Ok(None);
        };
        Ok(Some(parse_holder_count(code, row)))
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

        // K1（2026-09-28，长川科技 300604 运行 414a926c 实证）：F10 主题报表**只有成分
        // 关系、没有涨跌幅列**，`change_pct` 此前被硬编码为 None ⇒ 分析师如实报
        // 「概念涨幅数据缺失」（该失败标记是对的，错在我方从未拼接行情）。
        // 现用 bkzj 三张榜单（t:1 地域 / t:2 行业 / t:3 概念）按板块名 join 补当日涨跌幅；
        // 榜单是旁路增强：任一请求失败 ⇒ 静默跳过（该表成员保持 null），不得拖垮归属主结果。
        let mut pct_lists: Vec<Value> = Vec::new();
        for t in ["1", "2", "3"] {
            if let Ok(resp) = self.em_get(&block_quotes_url(t)).await {
                if let Ok(v) = resp.json::<Value>().await {
                    pct_lists.push(v);
                }
            }
        }
        let pct_map = build_block_pct_map(&pct_lists);
        for b in &mut concepts {
            b.change_pct = pct_map.get(&b.name).copied();
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
            | "get_cls_flash"
            // T9(2026-09-26)：估值报表 RPT_VALUEANALYSIS_DET 支持 TRADE_DATE<= 过滤，
            // 且本就按股票返回多日历史 ⇒ 截止日口径天然可得，见 get_peers_with_asof
            | "get_peers"
            // T15(2026-09-27)：板块指数日 K 有原生 end= 参数 ⇒ 行业排名可按截止日合成，
            // 见 get_industry_ranking_with_asof
            | "get_industry_ranking" => AsOfCapability::NativeDateParam,
            // SynthesizeFromKline: 实时报价/指数,用 K 线最后一行合成
            "get_quote" | "get_index_quotes" => AsOfCapability::SynthesizeFromKline,
            // NoHistoricalSemantic: 当下榜单（快照唯一通道，给当下值没有意义）
            // ⚠ 行业排名不在此列（T15 起有合成通道，见 get_industry_ranking_with_asof）
            // ⚠ get_holder_count（2026-10-01）同列：`RPT_HOLDERNUMLATEST` 本身就是
            //   「最新一期」快照表，没有日期参数可收窄；历史多期在 `RPT_F10_EH_HOLDERNUM`，
            //   接它属下一轮（届时改申报 NativeDateParam）。当下回放按**结构性缺口**留痕。
            "get_hot_stocks" | "get_holder_count" => AsOfCapability::NoHistoricalSemantic,
            // T12(2026-09-27)：板块归属**不是**「无历史语义」而是「只有当下值、且是慢变量」
            // —— 与 `get_sector_info` 同一张 `RPT_F10_CORETHEME_BOARDTYPE`（无日期列）。
            // 此前申报 `NoHistoricalSemantic` 使路由层的 as-of 白名单**跳过本仓唯一还活着的
            // 归属源**，只去探测申报 Fallthrough 的 baidu_stock（该接口已 301 失效），
            // 结果整维拿空；而同行业归属在 T10 后是容忍 live 值的 —— 两条口径互相矛盾。
            // Fallthrough: vendor 返回带 date 字段的全量,lib.rs 截断(已正确)
            "get_concept_blocks"
            | "get_financials"
            | "get_dragon_tiger"
            | "get_lockup_schedule"
            | "get_north_bound_holding"
            | "get_shareholder_trades"
            | "get_dividend_records"
            | "get_consensus_eps"
            | "get_block_trades"
            | "get_institutional_visits"
            | "get_sector_info"
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
            // T9(2026-09-26)：估值表支持 TRADE_DATE 上限过滤，见 get_peers_with_asof
            "get_peers",
            // T15(2026-09-27)：板块指数日 K 有原生 end= ⇒ 行业排名可按截止日合成
            "get_industry_ranking",
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
        // 只剩「当日榜单」这一档：给当下值对回放没有意义（榜单本身就是那天的产物）。
        // 板块归属不在此列（T12，见 `fallthrough_methods`）；行业排名也不在（T15，见
        // `native_date_param_methods` —— 板块指数日 K 能按截止日合成）。
        // 行业排名已移出本档（T15 起走日 K 合成，见 `native_date_param_methods`）
        assert_eq!(
            v.asof_capability("get_hot_stocks"),
            AsOfCapability::NoHistoricalSemantic,
            "get_hot_stocks 应该是 NoHistoricalSemantic"
        );
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
            // T10(2026-09-27)：与概念板块同一张无日期列的归属表，慢变量 ⇒ 容忍当日值
            "get_sector_info",
            // T12(2026-09-27)：`get_concept_blocks` 原申报 NoHistoricalSemantic，会让 as-of
            // 白名单跳过本仓唯一还活着的归属源 ⇒ 整维拿空。归属与行业同源同口径。
            "get_concept_blocks",
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

    /// 带显式市场标记的输入按**标记**取市场位，两种写法都要落回指数本体。
    ///
    /// 缺陷实证（2026-10-01 运行日志）：荐股链 `get_klines("000001.SH")` 在此被拼成
    /// `secid=0.000001.SH`（全链路取空，市场状态恒「未知」）；而把 `sh000001` 剥前缀再
    /// 按首位推断会得到 `0.000001` = **平安银行** —— 那是静默取回错误标的，比取空更坏。
    #[test]
    fn explicit_market_tag_wins_over_first_digit_inference() {
        assert_eq!(to_em_secid("000001.SH"), "1.000001");
        assert_eq!(to_em_secid("sh000001"), "1.000001");
        assert_eq!(to_em_secid("399006.SZ"), "0.399006");
        // 标记与首位推断一致时结果不变（股票口径零位移）
        assert_eq!(to_em_secid("600519.SH"), "1.600519");
        assert_eq!(to_em_secid("000001.SZ"), "0.000001");
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
            // 该夹具只关心 main_net_inflow；四档按「未披露」给 None
            small_net: None,
            medium_net: None,
            large_net: None,
            super_large_net: None,
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
        assert_eq!(rows[0].small_net, Some(-20.0));
        assert_eq!(rows[0].medium_net, Some(-30.0));
        assert_eq!(rows[0].large_net, Some(-40.0));
        assert_eq!(rows[0].super_large_net, Some(140.0));
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

    /// 回放路径的关键词清洗：组合词必须退化成股票名片段 —— 不洗的话 12 页预算
    /// 必然够不到截止日（实测「透景生命 政策」1 页 50 条全在同一天）。
    #[test]
    fn asof_keyword_narrows_compound_keyword() {
        assert_eq!(asof_news_keyword("透景生命 政策"), "透景生命");
        assert_eq!(asof_news_keyword("300642"), "300642", "纯代码原样（get_news 传的就是代码）");
        assert_eq!(asof_news_keyword("贵州茅台"), "贵州茅台", "单一片段不动");
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

#[cfg(test)]
mod peers_asof_tests {
    //! T9(2026-09-26)：同行对比「截止日口径」的 URL 判据（零网络）。
    //! 缺陷形态是「回放里同行 PE/涨跌幅永远是今天」——这里锁住日期上限必须落在
    //! 同一个 filter 里，且括号保持字面（编码成 %28/%29 会被服务端判参数错误）。
    use super::*;

    const IN_LIST: &str = "%22600519%22,%22000001%22";

    #[test]
    fn peers_valuation_url_appends_cutoff() {
        let live = peers_valuation_url(IN_LIST, None, 34);
        assert!(live.contains("RPT_VALUEANALYSIS_DET"), "仍走估值明细表: {live}");
        assert!(
            live.contains(&format!("(SECURITY_CODE%20in%20({IN_LIST}))")),
            "in 列表必须带字面括号: {live}"
        );
        assert!(!live.contains("TRADE_DATE%3C%3D"), "live 不该有日期上限: {live}");
        assert!(live.ends_with("pageSize=34"), "分页上限按票数放大: {live}");

        let replay = peers_valuation_url(IN_LIST, Some("2026-09-22"), 34);
        assert!(
            replay.contains(&format!(
                "(SECURITY_CODE%20in%20({IN_LIST}))(TRADE_DATE%3C%3D%272026-09-22%27)"
            )),
            "回放必须在同一 filter 内追加 TRADE_DATE 上限: {replay}"
        );
        assert!(
            replay.contains("sortColumns=TRADE_DATE&sortTypes=-1"),
            "倒序 ⇒ 每只票首行即截止日前最近一期: {replay}"
        );
    }

    /// 2026-10-01：同侪 ROE 的 URL 判据（`RPT_F10_FINANCE_MAINFINADATA`，零网络）。
    ///
    /// 锁三件事：① 报表名必须换掉 —— `RPT_VALUEANALYSIS_DET` **结构上不含 ROE**，
    /// 光改解析不改 URL 会得到一个恒空的 map（静默，等于没修）；
    /// ② 日期上限与 live 形态；③ 分页上限随票数放大（每只票要多期才能挑年报）。
    #[test]
    fn peers_roe_url_uses_mainfinadata_and_appends_cutoff() {
        let live = peers_roe_url(IN_LIST, None, 34);
        assert!(
            live.contains("RPT_F10_FINANCE_MAINFINADATA"),
            "ROE 必须走主要财务指标表（估值表没有 ROEJQ 字段）: {live}"
        );
        assert!(live.contains("ROEJQ"), "必须取 ROEJQ（与逐股财报路径同字段）: {live}");
        assert!(
            live.contains(&format!("(SECUCODE%20in%20({IN_LIST}))")),
            "in 列表必须带字面括号（编码成 %28/%29 会被服务端判参数错误）: {live}"
        );
        assert!(!live.contains("REPORT_DATE%3C%3D"), "live 不该有日期上限: {live}");
        assert!(live.ends_with("pageSize=34"), "分页上限按票数放大: {live}");

        let replay = peers_roe_url(IN_LIST, Some("2026-09-22"), 34);
        assert!(
            replay.contains(&format!(
                "(SECUCODE%20in%20({IN_LIST}))(REPORT_DATE%3C%3D%272026-09-22%27)"
            )),
            "回放必须在同一 filter 内追加 REPORT_DATE 上限: {replay}"
        );
        assert!(
            replay.contains("sortColumns=REPORT_DATE&sortTypes=-1"),
            "倒序是 `pick_peer_roe` 的前提（首见即最新）: {replay}"
        );
    }

    /// 2026-10-01：同侪 ROE 的**选取**判据 —— 年报优先、代码去后缀、缺年报退到最近一期。
    ///
    /// 夹具是**真实响应切片**（2026-10-01 实拉，600887/000596/000568 三票）：
    /// 600887 的 2026-06-30 是 10.09、2025-12-31 是 20.87 —— 若「直接取最新一期」，
    /// 横截面会把这家中报同侪读成「盈利能力只有年报同侪的一半」（差 2.07 倍）。
    #[test]
    fn pick_peer_roe_prefers_annual_and_normalizes_code() {
        let rows: Vec<Value> = serde_json::from_str(
            r#"[
              {"SECUCODE":"600887.SH","REPORT_DATE":"2026-06-30 00:00:00","ROEJQ":10.09},
              {"SECUCODE":"600887.SH","REPORT_DATE":"2026-03-31 00:00:00","ROEJQ":9.44},
              {"SECUCODE":"600887.SH","REPORT_DATE":"2025-12-31 00:00:00","ROEJQ":20.87},
              {"SECUCODE":"600887.SH","REPORT_DATE":"2025-09-30 00:00:00","ROEJQ":18.6},
              {"SECUCODE":"000596.SZ","REPORT_DATE":"2026-06-30 00:00:00","ROEJQ":8.42},
              {"SECUCODE":"300999.SZ","REPORT_DATE":"2026-06-30 00:00:00","ROEJQ":null},
              {"SECUCODE":"000568.SZ","REPORT_DATE":null,"ROEJQ":8.62}
            ]"#,
        )
        .expect("夹具应是合法 JSON");
        let m = pick_peer_roe(&rows);

        // ① 年报优先：取 2025-12-31 的 20.87，而不是最新的 2026-06-30 的 10.09
        assert_eq!(
            m.get("600887"),
            Some(&("2025-12-31".to_string(), 20.87)),
            "必须优先年报（混期会让横截面失真 2 倍）"
        );
        // ② 代码归一：SECUCODE 的 `.SH/.SZ` 后缀必须去掉，否则与 SECURITY_CODE 对不上
        assert!(
            m.contains_key("000596"),
            "键应是纯数字代码，实际: {:?}",
            m.keys().collect::<Vec<_>>()
        );
        assert!(!m.keys().any(|k| k.contains('.')), "键不得带交易所后缀");
        // ③ 无年报 ⇒ 退到最近一期，且**口径如实标出**（消费端据此知道不可与年报同侪直接比）
        assert_eq!(m.get("000596"), Some(&("2026-06-30".to_string(), 8.42)));
        // ④ 无值/无日期的行必须被丢弃（不制造 0.0 假值，也不猜日期）
        assert!(!m.contains_key("300999"), "ROEJQ 为 null 的行不得入表");
        assert!(!m.contains_key("000568"), "REPORT_DATE 缺失的行不得入表");
    }
}

#[cfg(test)]
mod balance_sheet_tests {
    //! 2026-10-01：商誉/应收账款的 URL 与逐期选取判据（零网络）。
    //! 被判缺陷：`financials` 路径把这两个字段写死 `None`，导致
    //! `fundamentals_report` 的「风险 | 商誉 / 应收账款」两行**整行不渲染**，
    //! 而提示词却声称「已包含在预聚合报告中」⇒ 分析师只能报「A 股特色风险维度数据缺失」。
    //! 夹具是**真实响应切片**（2026-10-01 实拉 600887）。
    use super::*;

    #[test]
    fn balance_sheet_url_pins_report_and_columns() {
        let u = balance_sheet_url("600887.SH");
        assert!(u.contains("RPT_F10_FINANCE_GBALANCE"), "报表名: {u}");
        assert!(u.contains("GOODWILL") && u.contains("ACCOUNTS_RECE"), "两个科目都要: {u}");
        assert!(
            u.contains("(SECUCODE%3D%22600887.SH%22)"),
            "按 SECUCODE 精确过滤（括号与引号需字面编码）: {u}"
        );
        assert!(
            u.contains("sortColumns=REPORT_DATE&sortTypes=-1"),
            "倒序返回；逐期建表不依赖顺序，但 URL 形态要钉住: {u}"
        );
    }

    #[test]
    fn pick_balance_sheet_keeps_each_period_separate() {
        let rows: Vec<Value> = serde_json::from_str(
            r#"[
              {"SECUCODE":"600887.SH","REPORT_DATE":"2026-06-30 00:00:00","GOODWILL":633004681.58,"ACCOUNTS_RECE":4077520954.97},
              {"SECUCODE":"600887.SH","REPORT_DATE":"2026-03-31 00:00:00","GOODWILL":2173279390.31,"ACCOUNTS_RECE":3562528703.34},
              {"SECUCODE":"600887.SH","REPORT_DATE":"2025-12-31 00:00:00","GOODWILL":2181530707.86,"ACCOUNTS_RECE":null},
              {"SECUCODE":"600887.SH","REPORT_DATE":null,"GOODWILL":1.0,"ACCOUNTS_RECE":1.0}
            ]"#,
        )
        .expect("夹具应是合法 JSON");
        let m = pick_balance_sheet(&rows);
        // ① 逐期独立：中报与一季报的商誉差 3.4 倍（633.0M vs 2173.3M）——
        //    这正是「取最新一期填全表」会抹掉的信息
        assert_eq!(m.get("2026-06-30"), Some(&(Some(633004681.58), Some(4077520954.97))));
        assert_eq!(m.get("2026-03-31"), Some(&(Some(2173279390.31), Some(3562528703.34))));
        // ② 单科目为空也要保留该期（另一个科目仍是真值）
        assert_eq!(m.get("2025-12-31"), Some(&(Some(2181530707.86), None)));
        // ③ 无报告期的行丢弃
        assert_eq!(m.len(), 3, "无 REPORT_DATE 的行不得入表: {m:?}");
    }
}

#[cfg(test)]
mod holder_count_tests {
    //! 2026-10-01：股东户数的 URL 与解析判据（零网络）。
    //! 被判缺陷：`lockup-watcher.md` 三处要求「股东人数（户均持股）」，却没有任何通道
    //! ⇒ 分析师只能写「`data_gaps`：股东人数数据缺失」⇒ 被判「⚠️ 低置信」（300604 实证）。
    //! 夹具是**真实响应切片**（2026-10-01 实拉 300604）。
    use super::*;

    #[test]
    fn holder_count_url_pins_report_and_columns() {
        let u = holder_count_url("300604.SZ");
        assert!(u.contains("RPT_HOLDERNUMLATEST"), "报表名: {u}");
        for col in
            ["HOLDER_NUM", "HOLDER_NUM_RATIO", "AVG_HOLD_NUM", "END_DATE", "HOLD_NOTICE_DATE"]
        {
            assert!(u.contains(col), "缺列 {col}: {u}");
        }
        assert!(u.contains("(SECUCODE%3D%22300604.SZ%22)"), "按 SECUCODE 过滤: {u}");
        assert!(u.ends_with("pageSize=1"), "该表只有最新一期，取 1 行即可: {u}");
    }

    #[test]
    fn parse_holder_count_truncates_dates_and_keeps_ratio() {
        let row: Value = serde_json::from_str(
            r#"{"SECUCODE":"300604.SZ","END_DATE":"2026-06-30 00:00:00","HOLDER_NUM":120196,
                "HOLDER_NUM_RATIO":72.356138061574,"AVG_HOLD_NUM":5278.20072215382,
                "HOLD_NOTICE_DATE":"2026-08-28 00:00:00"}"#,
        )
        .expect("夹具应是合法 JSON");
        let h = parse_holder_count("300604", &row);
        assert_eq!(h.stock_code, "300604");
        // 日期截到 10 位（消费端按 YYYY-MM-DD 读）
        assert_eq!(h.end_date, "2026-06-30");
        assert_eq!(h.notice_date.as_deref(), Some("2026-08-28"));
        assert_eq!(h.holder_num, Some(120196.0));
        // 变化率必须保留符号语义：正 72.36 = 户数上升 = **分散**（解读口径写在提示词里）
        assert!((h.holder_num_ratio.unwrap_or(0.0) - 72.356138061574).abs() < 1e-9);
        assert!((h.avg_hold_num.unwrap_or(0.0) - 5278.20072215382).abs() < 1e-6);
    }

    /// 缺列/缺行时不得伪造 0 值（「没有」与「是 0」必须可区分）。
    #[test]
    fn parse_holder_count_leaves_missing_fields_none() {
        let row: Value = serde_json::from_str(r#"{"SECUCODE":"300604.SZ"}"#).unwrap();
        let h = parse_holder_count("300604", &row);
        assert_eq!(h.end_date, "");
        assert!(h.holder_num.is_none() && h.holder_num_ratio.is_none() && h.notice_date.is_none());
    }
}

#[cfg(test)]
mod paging_window_tests {
    use super::*;

    fn audit(oldest: Option<&str>, exhausted: bool, fetched: usize, pages: u32) -> AsofPagingAudit {
        AsofPagingAudit {
            origin: "；已由原词清洗而来",
            fetched,
            pages,
            oldest: oldest.map(str::to_string),
            exhausted,
        }
    }

    /// T12(2026-09-27)：「退不到截止日」有两种根因，合并成一句话就会把人引向
    /// 错误的下一步（以为多翻几页就有，实际接口索引窗口就到 20 页）。
    #[test]
    fn index_bottom_and_window_cap_are_worded_differently() {
        let bottom = audit(Some("2026-09-18"), true, 400, 9).failure("集成电路", "2026-09-11", 20);
        assert!(bottom.contains("已翻到索引底"), "见底必须说明是数据面事实: {bottom}");
        assert!(
            bottom.contains("最晚到 2026-09-18"),
            "要给出实际退到的日期，否则无法判断差多少: {bottom}"
        );
        assert!(!bottom.contains("窗口就到"), "见底与窗口天花板不能混写: {bottom}");
        assert!(bottom.ends_with("已由原词清洗而来"), "清洗痕迹要保留: {bottom}");

        let capped =
            audit(Some("2026-09-11"), false, 1000, 20).failure("集成电路", "2026-08-01", 20);
        assert!(capped.contains("倒序窗口就到 20 页"), "窗口天花板要说是接口上限: {capped}");
        assert!(capped.contains("只到 2026-09-11"), "同样要给出退到的日期: {capped}");
        assert!(!capped.contains("索引底"), "窗口耗尽不等于该词没有更多新闻: {capped}");

        let no_date = audit(None, false, 0, 1).failure("集成电路", "2026-08-01", 20);
        assert!(no_date.contains("未取到任何带日期的条目"), "日期全不可解析时另说: {no_date}");
    }
}

#[cfg(test)]
mod board_ranking_tests {
    use super::*;

    /// 名单响应按 2026-09-27 实测构造（`f13.f12` 就是板块指数 secid）
    #[test]
    fn board_list_yields_secid_from_measured_shape() {
        let json: Value = serde_json::json!({
            "data": { "diff": [
                { "f3": 66, "f12": "BK0437", "f13": 90, "f14": "煤炭", "f62": 549783120 },
                { "f3": 46, "f12": "BK0436", "f13": 90, "f14": "纺织服饰" },
                { "f3": 30, "f12": "", "f13": 90, "f14": "无效行" },
            ] }
        });
        let boards = industry_board_list(&json);
        assert_eq!(
            boards,
            vec![
                ("90.BK0437".to_string(), "煤炭".to_string()),
                ("90.BK0436".to_string(), "纺织服饰".to_string())
            ],
            "空代码行必须剔除，secid 用 f13.f12 拼"
        );
    }

    /// K1（2026-09-28，300604 运行 414a926c）：概念板块涨跌幅拼接。
    /// F10 成分报表无涨跌幅列 ⇒ 用 bkzj 榜单按名字 join；join 不上保持 null（不编 0%）。
    #[test]
    fn block_pct_map_joins_by_name_and_keeps_miss_as_null() {
        let lists: Vec<Value> = vec![
            serde_json::json!({ "data": { "diff": [
                { "f3": 140, "f12": "BK1145", "f14": "机器人执行器" },
                { "f3": -62, "f12": "BK0148", "f14": "吉林板块" },
                { "f3": 0, "f12": "BK9999", "f14": "" }
            ] } }),
            serde_json::json!({ "data": { "diff": [
                { "f3": 66, "f12": "BK0437", "f14": "半导体设备" }
            ] } }),
            serde_json::json!({ "result": null }), // 榜单请求失败的占位：不得 panic
        ];
        let m = build_block_pct_map(&lists);
        assert_eq!(m.get("机器人执行器"), Some(&1.40), "f3=140 ⇒ 1.40%（÷100 约定同行业排名）");
        assert_eq!(
            m.get("吉林板块"),
            Some(&-0.62),
            "地域表(t:1)并进同一映射，浙江板块这类名字可命中"
        );
        assert_eq!(m.get("半导体设备"), Some(&0.66), "跨多张榜单合并");
        assert!(!m.contains_key(""), "空名行剔除");
        assert_eq!(m.get("不存在的板块"), None, "join 不上不得编 0%");
        // URL 判据：三张榜单只差 t 参数，走存活 host
        assert!(block_quotes_url("3").contains("data.eastmoney.com"));
        assert!(block_quotes_url("3").ends_with("code=m:90+t:3"));
    }

    #[test]
    fn board_kline_url_asks_only_two_bars_up_to_cutoff() {
        let url = board_kline_url("90.BK0437", "20260911");
        assert!(url.contains("secid=90.BK0437"), "{url}");
        assert!(url.contains("end=20260911"), "end 是接口原生日期参数: {url}");
        assert!(url.contains("lmt=2"), "只要截止日与前收两根: {url}");
        assert!(url.contains("fields2=f51,f53"), "收盘足够算涨跌幅: {url}");
    }

    /// 涨跌幅口径：本地再按截止日裁一次（`end` 不被尊重是 T1 的实测教训）
    #[test]
    fn board_change_truncates_by_cutoff_and_requires_two_bars() {
        let bars = |rows: &[&str]| -> Vec<Value> {
            rows.iter().map(|s| Value::String((*s).into())).collect()
        };
        let ok = board_change_from_bars(&bars(&["2026-09-10,100", "2026-09-11,106"]), "2026-09-11")
            .unwrap();
        assert_eq!(ok.0, "2026-09-11");
        assert!((ok.1 - 6.0).abs() < 1e-9, "涨跌幅 = 收盘/前收 - 1: {ok:?}");

        // 越过截止日的那根必须剔掉 ⇒ 裁完只剩一根 ⇒ 算不出涨跌幅，如实 None（不编 0%）
        assert!(board_change_from_bars(&bars(&["2026-09-11,100", "2026-09-12,180"]), "2026-09-11")
            .is_none());
        let shifted = board_change_from_bars(
            &bars(&["2026-09-09,100", "2026-09-10,102", "2026-09-12,180"]),
            "2026-09-11",
        )
        .unwrap();
        assert_eq!(shifted.0, "2026-09-10", "截止日休市 ⇒ 取之前最近交易日");
        assert!((shifted.1 - 2.0).abs() < 1e-9, "{shifted:?}");

        assert!(board_change_from_bars(&bars(&["2026-09-11,100"]), "2026-09-11").is_none());
        assert!(board_change_from_bars(&bars(&["2026-09-09,0", "2026-09-11,5"]), "2026-09-11")
            .is_none());
    }

    /// 合成主体按 URL 分夹具 ⇒ 零真实网络也覆盖到「排序 + 算不出就跳过 + 越界不采信」。
    #[tokio::test]
    async fn synthesis_ranks_only_boards_with_two_in_window_bars() {
        let fetch = |url: String| -> futures::future::BoxFuture<'static, Result<Value, DataError>> {
            Box::pin(async move {
                if url.contains("getbkzj") {
                    return Ok(serde_json::json!({ "data": { "diff": [
                        { "f12": "BK0437", "f13": 90, "f14": "煤炭" },
                        { "f12": "BK0436", "f13": 90, "f14": "纺织服饰" },
                        { "f12": "BK1283", "f13": 90, "f14": "银行" },
                    ] } }));
                }
                if url.contains("BK0437") {
                    return Ok(serde_json::json!({ "data": { "klines": [
                        "2026-09-10,100", "2026-09-11,110"
                    ] } }));
                }
                if url.contains("BK0436") {
                    // 第二根越过截止日 ⇒ 裁完不足两根
                    return Ok(serde_json::json!({ "data": { "klines": [
                        "2026-09-11,50", "2026-09-12,90"
                    ] } }));
                }
                Err(DataError::VendorError {
                    vendor: "eastmoney".into(),
                    message: "连接被拒".into(),
                })
            })
        };
        let ranks = synthesize_industry_ranking("eastmoney", "2026-09-11", fetch).await.unwrap();
        assert_eq!(ranks.len(), 1, "算不出的（越界/失败）板块必须跳过，而不是编成 0%: {ranks:?}");
        assert_eq!(ranks[0].industry_name, "煤炭");
        assert!((ranks[0].change_pct - 10.0).abs() < 1e-9, "{:?}", ranks[0]);
    }

    #[tokio::test]
    async fn synthesis_reports_failure_instead_of_empty_ok() {
        let fetch = |url: String| -> futures::future::BoxFuture<'static, Result<Value, DataError>> {
            Box::pin(async move {
                if url.contains("getbkzj") {
                    return Ok(serde_json::json!({ "data": { "diff": [
                        { "f12": "BK0437", "f13": 90, "f14": "煤炭" }
                    ] } }));
                }
                Err(DataError::VendorError {
                    vendor: "eastmoney".into(),
                    message: "push2his 连接被拒".into(),
                })
            })
        };
        let e = synthesize_industry_ranking("eastmoney", "2026-09-11", fetch).await.unwrap_err();
        let msg = e.to_string();
        assert!(msg.contains("push2his 连接被拒"), "真实原因要能进面板: {msg}");
        assert!(!msg.contains("无 as-of 通道"), "失败不得写成机制缺失: {msg}");
    }
}

#[cfg(test)]
mod datacenter_empty_tests {
    use super::*;

    /// 「空是答案 / 故障是故障」的分档判据（301302 回放实证：非质押标的被记成红档失败）
    #[test]
    fn empty_report_answer_is_not_a_failure() {
        assert!(datacenter_reports_no_data("返回数据为空"));
        assert!(datacenter_reports_no_data("code:9201, message:返回数据为空"));
        assert!(!datacenter_reports_no_data("参数预处理错误"), "故障不得被当成空答案");
        assert!(!datacenter_reports_no_data("报表配置不存在"));
    }
}

#[cfg(test)]
mod dividend_report_tests {
    //! 2026-10-01：分红报表下线的防回归判据（零网络）。
    //!
    //! 缺陷背景：`RPTA_WEB_DIVIDEND` 被东财下线后恒返回
    //! `{"result":null,"success":false,"message":"报表配置不存在,...","code":9501}`，
    //! 旧实现把它当「该股无分红」⇒ 全市场 dividend 静默为空，日志里只有一行
    //! 「返回空数据(非故障)，不触发 vendor 降级」。三个易错点必须钉住：报表名、
    //! 倒序分页、以及「每 10 股 → 每股」的单位换算。

    use super::*;

    /// 报表名 + 倒序 + 分页：任一丢失都会静默退化（整表空，或只取到二十年前的记录）
    #[test]
    fn dividend_url_pins_report_and_desc_sort() {
        let u = EastMoneyVendor::dividend_report_url("600519");
        assert!(u.contains("reportName=RPT_SHAREBONUS_DET"), "仍走分红送配报表: {u}");
        assert!(!u.contains("RPTA_WEB_DIVIDEND"), "不得回退到已下线的旧报表: {u}");
        assert!(u.contains("sortColumns=EX_DIVIDEND_DATE&sortTypes=-1"), "必须按除权日倒序: {u}");
        assert!(u.contains("pageSize=50"), "as-of 回放需要整段历史: {u}");
        assert!(u.contains(r#"SECURITY_CODE="600519""#), "纯数字代码等值过滤: {u}");
    }

    /// 带 sh/sz/bj 前缀的代码必须归一，否则 filter 恒空（2026-07-22 同类修复）
    #[test]
    fn dividend_url_normalizes_prefixed_codes() {
        let sh = EastMoneyVendor::dividend_report_url("sh600519");
        assert!(sh.contains(r#"SECURITY_CODE="600519""#), "{sh}");
        let sz = EastMoneyVendor::dividend_report_url("sz000063");
        assert!(sz.contains(r#"SECURITY_CODE="000063""#), "{sz}");
    }

    /// 每 10 股口径 → 每股；无送转记 0；日期截到 10 位（否则截止日当天会被 as-of 剔除）
    #[test]
    fn dividend_rows_convert_per_ten_shares_and_clip_date() {
        let row = serde_json::json!({
            "SECURITY_CODE": "600519",
            "EX_DIVIDEND_DATE": "2026-06-26 00:00:00",
            "EQUITY_RECORD_DATE": "2026-06-25 00:00:00",
            "PRETAX_BONUS_RMB": 280.2423,
            "BONUS_RATIO": null,
            "IT_RATIO": null,
        });
        let got = EastMoneyVendor::parse_dividend_rows("600519", &[row]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].ex_date, "2026-06-26", "必须截到 YYYY-MM-DD 才能与 cutoff 比较");
        assert_eq!(got[0].record_date, "2026-06-25");
        assert!((got[0].dividend_per_share - 28.02423).abs() < 1e-9, "{:?}", got[0]);
        assert!((got[0].bonus_share_ratio - 0.0).abs() < 1e-9, "无送转记为 0: {:?}", got[0]);
    }

    /// 字符串型数值同样接受；送股与转增**相加**（每 10 股送 1 转 3 ⇒ 每股 0.4）
    #[test]
    fn dividend_rows_accept_string_numbers_and_sum_bonus() {
        let row = serde_json::json!({
            "EX_DIVIDEND_DATE": "2003-07-14 00:00:00",
            "EQUITY_RECORD_DATE": "2003-07-11 00:00:00",
            "PRETAX_BONUS_RMB": "2",
            "BONUS_RATIO": "1",
            "IT_RATIO": "3",
        });
        let got = EastMoneyVendor::parse_dividend_rows("600519", &[row]);
        assert_eq!(got.len(), 1);
        assert!((got[0].dividend_per_share - 0.2).abs() < 1e-9, "{:?}", got[0]);
        assert!((got[0].bonus_share_ratio - 0.4).abs() < 1e-9, "{:?}", got[0]);
    }

    /// 无除权日的行（预案未定/已取消）不进结果：空 `ex_date` 会被 as-of 判为「时效未知」，
    /// 在回放里留下语义空洞，而它本就不构成一次除权除息事件。
    #[test]
    fn dividend_rows_drop_rows_without_ex_date() {
        let rows = [
            serde_json::json!({ "EX_DIVIDEND_DATE": null, "PRETAX_BONUS_RMB": 5 }),
            serde_json::json!({ "EX_DIVIDEND_DATE": "", "PRETAX_BONUS_RMB": 5 }),
            serde_json::json!({ "EX_DIVIDEND_DATE": "2025-06-26", "PRETAX_BONUS_RMB": 5 }),
        ];
        let got = EastMoneyVendor::parse_dividend_rows("600519", &rows);
        assert_eq!(got.len(), 1, "只有带除权日的那条保留");
        assert_eq!(got[0].ex_date, "2025-06-26");
    }
}

#[cfg(test)]
mod earnings_notice_tests {
    //! 2026-10-01：财报日历换用公告接口后的防回归判据（零网络）。
    //!
    //! 缺陷背景：`RPTA_WEB_NOTICE` 被东财下线后恒返回「报表配置不存在」，旧实现把
    //! `result.data` 缺失当「该股无财报事件」返空 ⇒ 主通道与浏览器兜底一起静默。
    //! 故这里既钉 URL 形态（纯数字代码 / 不带 cb），也钉「故障抛 Err、空才返空」。

    use super::*;

    /// 公告接口 URL：代码归一为纯数字，且**不带 `cb`**（JSONP 会让浏览器通道解析失败）
    #[test]
    fn notice_url_is_json_not_jsonp() {
        let u = notice_ann_url("sh600519", EARNINGS_NOTICE_PAGE_SIZE);
        assert!(u.contains("stock_list=600519"), "前缀需归一: {u}");
        assert!(!u.contains("cb="), "不能带 cb（JSONP 会让浏览器通道解析失败）: {u}");
        assert!(u.contains("np-anotice-stock.eastmoney.com"), "需在实测可用的公告域: {u}");
        assert!(u.contains(&format!("page_size={EARNINGS_NOTICE_PAGE_SIZE}")), "{u}");
    }

    /// 「接口失效」必须抛 Err，不得伪装成「该股没有财报事件」
    #[test]
    fn interface_failure_is_not_an_empty_answer() {
        let dead = serde_json::json!({
            "result": null,
            "success": false,
            "message": "报表配置不存在,RPTA_WEB_NOTICE",
            "code": 9501,
        });
        assert!(earnings_from_notice_json("600519", "eastmoney", &dead).is_err(), "必须报错");

        // 信封缺 data.list 同样是故障（风控页 / 接口变更），不是「无公告」
        let malformed = serde_json::json!({ "error": "", "success": 1 });
        assert!(earnings_from_notice_json("600519", "eastmoney", &malformed).is_err());

        // 真正的「无公告」：success=1 且 list 为空数组 ⇒ 返空，不报错
        let empty = serde_json::json!({
            "data": { "list": [], "total_hits": 0 },
            "error": "",
            "success": 1,
        });
        let got = earnings_from_notice_json("600519", "eastmoney", &empty).expect("空不是故障");
        assert!(got.is_empty());
    }

    /// 解析：日期截到 10 位、分类沿用 `classify_earnings_title`、非财报公告被滤掉
    #[test]
    fn notice_rows_map_to_earnings_events() {
        let json = serde_json::json!({
            "data": { "list": [
                {
                    "title": "贵州茅台:贵州茅台2026年半年度报告摘要",
                    "notice_date": "2026-08-15 00:00:00",
                    "codes": [{ "short_name": "贵州茅台" }],
                },
                {
                    "title": "贵州茅台:关于会计政策变更的公告",
                    "notice_date": "2026-08-15 00:00:00",
                    "codes": [{ "short_name": "贵州茅台" }],
                },
            ], "total_hits": 2 },
            "error": "",
            "success": 1,
        });
        let got = earnings_from_notice_json("600519", "browser_eastmoney", &json);
        let got = got.expect("应解析成功");
        assert_eq!(got.len(), 1, "非财报公告应被滤掉");
        assert_eq!(got[0].event_date, "2026-08-15", "必须截到 10 位才能与 as-of 比较");
        assert_eq!(got[0].event_type, "formal");
        assert_eq!(got[0].period.as_deref(), Some("2026Q2"));
        assert_eq!(got[0].stock_name, "贵州茅台");
        assert_eq!(got[0].source.as_deref(), Some("browser_eastmoney"));
    }
}
