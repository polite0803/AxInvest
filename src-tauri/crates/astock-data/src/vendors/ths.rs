use crate::as_of_capability::AsOfCapability;
use crate::error::DataError;
use crate::types::*;
use crate::vendors::StockVendor;
use async_trait::async_trait;
use serde_json::Value;

pub struct ThsVendor {
    pub http: reqwest::Client,
}

fn val_to_f64(v: &Value) -> Option<f64> {
    v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_f64())
}

/// 去除 sh/sz/bj 前缀（大小写不敏感），同花顺 URL 只接受纯数字代码
///
/// 修复(2026-07-22): 原代码直接用 stock_code 构造 URL,如传入 "sh600887",
/// URL 变为 `https://basic.10jqka.com.cn/sh600887/concept.shtml` 会 404。
/// 修复(2026-07-29): 大小写不敏感处理。原 `trim_start_matches("sh")` 只能去小写前缀,
///   传入 "SH600887" 时无法去除 → URL 仍 404。改为按 ASCII 大小写归一后判断。
fn pure_code(stock_code: &str) -> &str {
    // 找到第一个数字字符的位置（A股代码均为数字开头）
    // 这样可兼容 sh/sz/bj/SH/SZ/BJ 各种大小写前缀，无前缀时原样返回
    if let Some(idx) = stock_code.find(|c: char| c.is_ascii_digit()) {
        // 仅当存在合法前缀（前缀长度 ≤ 2 且非数字）时才截断
        let prefix = &stock_code[..idx];
        if prefix.is_empty()
            || prefix.eq_ignore_ascii_case("sh")
            || prefix.eq_ignore_ascii_case("sz")
            || prefix.eq_ignore_ascii_case("bj")
        {
            return &stock_code[idx..];
        }
    }
    stock_code
}

#[async_trait]
impl StockVendor for ThsVendor {
    async fn get_quote(&self, _: &str) -> Result<StockQuote, DataError> {
        Err(DataError::VendorError {
            vendor: "ths".into(),
            message: "quote handled by tencent vendor".into(),
        })
    }

    async fn get_klines(
        &self,
        _: &str,
        _: &str,
        _: u32,
        _: Option<AdjType>,
    ) -> Result<Vec<KLine>, DataError> {
        Ok(vec![])
    }

    async fn get_financials(&self, _: &str) -> Result<Vec<FinancialReport>, DataError> {
        Ok(vec![])
    }

    async fn get_news(&self, stock_code: &str, limit: u32) -> Result<Vec<NewsItem>, DataError> {
        // 同花顺个股新闻：https://basic.10jqka.com.cn/{code}/news.html
        let code = pure_code(stock_code);
        let url = format!(
            "https://basic.10jqka.com.cn/api/stockph.php?code={}&type=news&page=1&limit={}",
            code,
            limit.min(50)
        );
        let resp = self
            .http
            .get(&url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .header("Referer", format!("https://basic.10jqka.com.cn/{}/", code))
            .send()
            .await?;
        crate::check_response_429(&resp, "ths")?;

        let text = resp.text().await?;
        let json: Value = serde_json::from_str(&text).map_err(|e| {
            DataError::ParseError(format!(
                "ths news parse failed: {e}, raw: {}",
                &text[..text.len().min(120)]
            ))
        })?;

        // ths 返回格式: { "data": { "items": [...] } } 或 { "status_code": 0, "data": [...] }
        let items = json["data"]["items"]
            .as_array()
            .or_else(|| json["data"].as_array())
            .or_else(|| json["list"].as_array())
            .map(|a| a.to_vec())
            .unwrap_or_default();

        Ok(items
            .iter()
            .map(|item| {
                let title = item["title"]
                    .as_str()
                    .or_else(|| item["news_title"].as_str())
                    .unwrap_or("")
                    .to_string();
                let summary = item["summary"]
                    .as_str()
                    .or_else(|| item["content"].as_str())
                    .or_else(|| item["digest"].as_str())
                    .unwrap_or("")
                    .to_string();
                let source = item["source"]
                    .as_str()
                    .or_else(|| item["from"].as_str())
                    .unwrap_or("同花顺")
                    .to_string();
                let url = item["url"]
                    .as_str()
                    .or_else(|| item["news_url"].as_str())
                    .or_else(|| item["link"].as_str())
                    .unwrap_or("")
                    .to_string();
                let publish_time = item["publish_time"]
                    .as_str()
                    .or_else(|| item["date"].as_str())
                    .or_else(|| item["ctime"].as_str())
                    .unwrap_or("")
                    .to_string();

                NewsItem { title, summary, source, url, publish_time, sentiment_score: None }
            })
            .collect())
    }

    async fn get_money_flow(&self, _: &str) -> Result<Option<MoneyFlow>, DataError> {
        Ok(None)
    }

    async fn get_dragon_tiger(&self, _: &str) -> Result<Vec<DragonTigerEntry>, DataError> {
        Ok(vec![])
    }

    /// **本方法已停用**（P1-4，2026-10-03）：此前实现是拿涨停池**拼**成龙虎榜 ——
    /// `net_buy/buy_amount/sell_amount` 恒 0.0、`date` 空串、`reason` 取涨停原因，
    /// 而它曾被排在路由首位，于是「全市场龙虎榜」常年是伪记录且无法被读出是假的。
    /// 现在显式失败；要恢复请先接**真席位数据**（东财 `RPT_BILLBOARD_DAILYDETAILSBUY/SELL` 已有）。
    async fn get_market_dragon_tiger(&self) -> Result<Vec<MarketDragonTiger>, DataError> {
        Err(DataError::VendorError {
            vendor: "ths".into(),
            message: "该源没有龙虎榜口径（原实现由涨停池拼装，属伪记录 ⇒ 已停用）".into(),
        })
    }

    async fn get_lockup_schedule(&self, _: &str) -> Result<Vec<LockupSchedule>, DataError> {
        Ok(vec![])
    }

    async fn search_stock(&self, _: &str) -> Result<Vec<StockSearchResult>, DataError> {
        Ok(vec![])
    }

    async fn get_consensus_eps(&self, stock_code: &str) -> Result<Option<ConsensusEPS>, DataError> {
        let code = pure_code(stock_code);
        let url = format!("https://basic.10jqka.com.cn/{}/worth/", code);
        let resp = self
            .http
            .get(&url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .header("Referer", "https://basic.10jqka.com.cn/")
            .send()
            .await?;
        crate::check_response_429(&resp, "ths")?;

        let text = resp.text().await?;

        let eps = extract_json_between(&text, "var forecastData = ", ";")
            .and_then(|json_str| serde_json::from_str::<Value>(&json_str).ok());

        let eps_data = match eps {
            Some(v) => v,
            None => return Ok(None),
        };

        let items = match eps_data.as_array() {
            Some(arr) if !arr.is_empty() => arr,
            _ => return Ok(None),
        };

        let latest = &items[0];
        // 修复(2026-07-29): 原 `v.as_i64().map(|_| "")` 把数字年份（如 2024）映射成空字符串,
        //   导致 ConsensusEPS.year 永远为空。改为将数字年份转为字符串保留。
        let year = latest
            .get("year")
            .and_then(|v| {
                v.as_str()
                    .map(|s| s.to_string())
                    .or_else(|| v.as_i64().map(|i| i.to_string()))
                    .or_else(|| v.as_f64().map(|f| f.to_string()))
            })
            .unwrap_or_default();
        let consensus_eps = latest.get("avg").and_then(val_to_f64);
        let rating_count = latest.get("num").and_then(|v| {
            v.as_str().and_then(|s| s.parse::<i32>().ok()).or_else(|| v.as_i64().map(|i| i as i32))
        });

        if consensus_eps.is_none() && rating_count.is_none() {
            return Ok(None);
        }

        Ok(Some(ConsensusEPS {
            stock_code: stock_code.to_string(),
            consensus_eps,
            consensus_target_price: None,
            rating_avg: None,
            rating_count,
            year,
            // vendor 返回的真实一致预期 ⇒ 非估算
            is_estimated: false,
            estimate_source: None,
        }))
    }

    async fn get_concept_blocks(
        &self,
        stock_code: &str,
    ) -> Result<Option<ConceptBlocks>, DataError> {
        let code = pure_code(stock_code);
        let url = format!("https://basic.10jqka.com.cn/{}/concept.shtml", code);
        let resp = self
            .http
            .get(&url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .header("Referer", "https://basic.10jqka.com.cn/")
            .send()
            .await?;
        crate::check_response_429(&resp, "ths")?;

        let text = resp.text().await?;

        let blocks = extract_json_between(&text, "var conceptList = ", ";")
            .and_then(|json_str| serde_json::from_str::<Value>(&json_str).ok());

        let industry = extract_json_between(&text, "var industry = ", ";")
            .and_then(|json_str| serde_json::from_str::<Value>(&json_str).ok())
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_default();

        let concepts = match blocks {
            Some(arr) if arr.is_array() => arr
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|item| {
                    Some(BlockItem {
                        name: item.get("name")?.as_str()?.to_string(),
                        change_pct: item.get("change").and_then(val_to_f64),
                    })
                })
                .collect(),
            _ => vec![],
        };

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

    /// 热股榜（真·热度榜：同花顺 fuyao `hot_list`）。
    ///
    /// **名目收编(2026-10-03)**：本方法此前打的是 `dataapi/limit_up/limit_up_pool?limit=20`
    /// —— 涨停池的前 20 行，与 `get_limit_up_pool` 同一接口同一业务名目，而下游两处都按
    /// 「热度榜」读它（`HotStocksPanel` 的标题、`SocialSentiment.hot_rank` 的回填）。
    /// 涨停池归 `get_limit_up_pool`（字段更全且按日可回溯），这里换成真正的热股榜端点。
    ///
    /// 实测（2026-10-03）：`data.stock_list` 恒 100 行；`order` = 榜内名次、
    /// `rate` = 热度值（**字符串**，`type=hour` 与 `type=day` 量级差三个数量级，只可同期比）、
    /// `rise_and_fall` **已经是百分比原值**（601127 的 0.2777 对腾讯行情的 0.28%，
    /// 新股 001246 的 206.5944 对 206.59%）⇒ 不得再乘 100；`date` 参数不被采纳（回显恒为当下）
    /// ⇒ 能力申报维持 `NoHistoricalSemantic`。榜单不提供换手率与行业名，对应字段留 `None`。
    async fn get_hot_stocks(&self) -> Result<Vec<HotStock>, DataError> {
        let url = "https://dq.10jqka.com.cn/fuyao/hot_list_data/out/hot_list/v1/stock\
                   ?stock_type=a&type=hour&list_type=normal";
        let json = ths_get_json(&self.http, url).await?;
        let status = json.get("status_code").and_then(Value::as_i64);
        if status != Some(0) {
            let msg = json.get("status_msg").and_then(Value::as_str).unwrap_or("无");
            return Err(DataError::VendorError {
                vendor: "ths".into(),
                message: format!("get_hot_stocks 接口拒绝: status_code={status:?} msg={msg:?}"),
            });
        }
        let rows: Vec<Value> = json
            .get("data")
            .and_then(|d| d.get("stock_list"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        // 全市场热股榜不存在「今天一个都没有」这种答案：空即接口异常，
        // 按 Err 上报，避免下游把「取不到」读成「无人气」。
        if rows.is_empty() {
            return Err(DataError::VendorError {
                vendor: "ths".into(),
                message: "get_hot_stocks 返回空榜单（实测恒 100 行）⇒ 判接口异常".into(),
            });
        }
        Ok(rows
            .iter()
            .filter_map(|item| {
                let code = item.get("code")?.as_str()?.to_string();
                let name = item.get("name")?.as_str()?.to_string();
                let change_pct = item.get("rise_and_fall").and_then(val_to_f64).unwrap_or(0.0);
                let rank = item.get("order").and_then(Value::as_u64).map(|v| v as u32);
                let hot_value = item.get("rate").and_then(val_to_f64);
                let reason_tags = item
                    .get("tag")
                    .and_then(|t| t.get("concept_tag"))
                    .and_then(Value::as_array)
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                Some(HotStock {
                    stock_code: code,
                    stock_name: name,
                    change_pct,
                    // 热股榜不给换手率（真值在涨停池那边）—— 留 None，不用 0 顶替
                    turnover_rate: None,
                    reason_tags,
                    sector: None,
                    rank,
                    hot_value,
                })
            })
            .collect())
    }

    /// 涨停池（连板数 / 涨停天数 / 换手 / 封单 / 炸板次数 / 板型 / 涨停逻辑）。
    ///
    /// 实测（2026-10-03）`date` 参数**生效**且 `data.date` 回显请求日 ⇒ 本通道申报
    /// `NativeDateParam`，回放可直接按截止日取当日池（东财 push2ex 那条不采纳 date，已撤）。
    ///
    /// 分页：`limit` 必须 ≤ 200（实测 500 直接回 `status_code=-1`）。既有 `get_hot_stocks`
    /// 不查 `status_code`，那种「参数被拒」会被读成空列表 ⇒ 本方法显式判状态码。
    async fn get_limit_up_pool(
        &self,
        requested_date: Option<&str>,
    ) -> Result<Option<LimitUpPool>, DataError> {
        let date_compact = requested_date.map(|d| d.replace('-', ""));
        let mut entries: Vec<LimitUpPoolEntry> = Vec::new();
        let mut pool_date = String::new();
        let mut breadth: Option<LimitUpBreadth> = None;
        let mut reported_total: Option<i64> = None;
        let mut page: i64 = 1;
        let mut last_page: i64 = 1;
        loop {
            let url = match &date_compact {
                Some(d) => format!(
                    "{POOL_URL_BASE}page={page}&limit={POOL_PAGE_LIMIT}&field={POOL_FIELDS}&date={d}"
                ),
                None => format!(
                    "{POOL_URL_BASE}page={page}&limit={POOL_PAGE_LIMIT}&field={POOL_FIELDS}"
                ),
            };
            let json = ths_get_json(&self.http, &url).await?;
            pool_status_guard(&json, date_compact.as_deref())?;
            let data = json.get("data").and_then(Value::as_object).ok_or_else(|| {
                DataError::ParseError("ths 涨停池 status_code=0 但 data 缺失".into())
            })?;
            if page == 1 {
                pool_date = normalize_pool_date(data.get("date").and_then(Value::as_str))?;
                breadth = parse_limit_up_breadth(data);
                let pg = data.get("page");
                reported_total = pg.and_then(|p| p.get("total")).and_then(Value::as_i64);
                last_page = pg
                    .and_then(|p| p.get("count"))
                    .and_then(Value::as_i64)
                    .unwrap_or(1)
                    .clamp(1, POOL_MAX_PAGES);
            }
            let rows: Vec<Value> =
                data.get("info").and_then(Value::as_array).cloned().unwrap_or_default();
            let row_count = rows.len();
            entries.extend(rows.iter().filter_map(parse_pool_row));
            if page >= last_page || row_count == 0 {
                break;
            }
            page += 1;
        }
        // 半池伪装成全池 = 下游把「没数到的票」读成「没涨停」，故收不齐必须硬失败。
        if let Some(t) = reported_total {
            if t as usize != entries.len() {
                return Err(DataError::ParseError(format!(
                    "ths 涨停池分页未收齐：接口自报 total={t}，实收 {}（page={page}/last_page={last_page}）",
                    entries.len()
                )));
            }
        }
        let pool = LimitUpPool {
            pool_date,
            requested_date: requested_date.map(str::to_string),
            entries,
            breadth,
        };
        if !pool.is_for_requested_date() {
            return Err(DataError::ParseError(format!(
                "ths 涨停池回显日期 {} 不等于请求日 {:?} ⇒ 该源已不再按日期返回，继续用会把当下池当历史池",
                pool.pool_date, pool.requested_date
            )));
        }
        Ok(Some(pool))
    }

    async fn get_industry_ranking(&self) -> Result<Vec<IndustryRank>, DataError> {
        let url = "https://data.10jqka.com.cn/dataapi/limit_up/industry_board?page=1&limit=90&sort_field=change_pct&sort_order=desc&field=199112,10,9001,330323,330324,330325,9002,330329,133971,133970,1968584,3475914,9003,9004";
        let resp = self
            .http
            .get(url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .header("Referer", "https://data.10jqka.com.cn/")
            .send()
            .await?;
        crate::check_response_429(&resp, "ths")?;

        let json: Value = resp.json().await?;
        let data = &json["data"];
        if data.is_null() {
            return Err(DataError::VendorError {
                vendor: "ths".into(),
                message: "get_industry_ranking 数据为空".into(),
            });
        }

        let empty_vec2 = vec![];
        let ranks = data
            .as_object()
            .and_then(|obj| obj.get("info").or_else(|| obj.get("list")))
            .and_then(|v| v.as_array())
            .or_else(|| data.as_array())
            .unwrap_or(&empty_vec2);

        Ok(ranks
            .iter()
            .filter_map(|item| {
                let industry_name = item
                    .get("industry_name")
                    .or_else(|| item.get("name"))
                    .and_then(|v| v.as_str())?
                    .to_string();
                let change_pct = item
                    .get("change_rate")
                    .or_else(|| item.get("change_pct"))
                    .and_then(val_to_f64)
                    .unwrap_or(0.0);
                let turnover =
                    item.get("turnover").or_else(|| item.get("amount")).and_then(val_to_f64);
                let leader_code = item
                    .get("leader_code")
                    .or_else(|| item.get("code"))
                    .and_then(|v| v.as_str().map(|s| s.to_string()));
                let leader_name =
                    item.get("leader_name").and_then(|v| v.as_str().map(|s| s.to_string()));
                let leader_change_pct = item.get("leader_change_pct").and_then(val_to_f64);

                Some(IndustryRank {
                    industry_name,
                    change_pct,
                    turnover,
                    main_inflow: None,
                    leader_code,
                    leader_name,
                    leader_change_pct,
                })
            })
            .collect())
    }

    async fn get_north_bound_flow(&self) -> Result<Option<NorthBoundFlow>, DataError> {
        let url = "https://data.10jqka.com.cn/dataapi/hsgt/hsgt_board";
        let resp = self
            .http
            .get(url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .header("Referer", "https://data.10jqka.com.cn/")
            .send()
            .await?;
        crate::check_response_429(&resp, "ths")?;

        let json: Value = resp.json().await?;
        let data = &json["data"];
        if data.is_null() {
            return Err(DataError::VendorError {
                vendor: "ths".into(),
                message: "get_north_bound_flow 数据为空".into(),
            });
        }

        let sh_flow =
            data.get("sh_flow").or_else(|| data.get("hgt")).and_then(val_to_f64).unwrap_or(0.0);
        let sz_flow =
            data.get("sz_flow").or_else(|| data.get("sgt")).and_then(val_to_f64).unwrap_or(0.0);

        Ok(Some(NorthBoundFlow {
            date: data
                .get("date")
                .or_else(|| data.get("tradedate"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            sh_flow,
            sz_flow,
            total_flow: sh_flow + sz_flow,
            timestamp: data.get("time").and_then(|v| v.as_str().map(|s| s.to_string())),
            recent_history: vec![],
        }))
    }

    // ── P3:ths 能力申报 ──
    // 真实实现:
    // - get_market_dragon_tiger:带 date 字段 → Fallthrough
    // - get_hot_stocks:当下榜单 → NoHistoricalSemantic
    // - get_industry_ranking:当下排名 → NoHistoricalSemantic
    // - get_north_bound_flow:带 date 字段 → Fallthrough
    // - get_consensus_eps:带 year 字段 → Fallthrough
    // - get_concept_blocks:当下概念分类 → NoHistoricalSemantic
    // - get_news:带 publish_time 字段 → Fallthrough
    // - get_limit_up_pool:带 date 且实测回显当日 → NativeDateParam
    // 其他 stub:Fallthrough
    fn asof_capability(&self, method: &str) -> AsOfCapability {
        match method {
            "get_hot_stocks" | "get_industry_ranking" | "get_concept_blocks" => {
                AsOfCapability::NoHistoricalSemantic
            },
            // P9-4(2026-10-03)：涨停池是本仓目前**唯一按日可回溯**的涨停通道 ——
            // 实测 2025-09-18 起至最新交易日逐日可取（`data.date` 恒等于请求日），
            // 越界日期回 `status_code=-1`（⇒ 判「不可得」），非交易日回 `total=0`（⇒ 判「真无」）。
            // 对照东财 `push2ex/getTopicZTPool` 的 `date` 不被采纳，故该实现已撤除。
            "get_limit_up_pool" => AsOfCapability::NativeDateParam,
            _ => AsOfCapability::Fallthrough,
        }
    }
}

const POOL_URL_BASE: &str = "https://data.10jqka.com.cn/dataapi/limit_up/limit_up_pool?";
/// 实测上限：`limit=500` 回 `status_code=-1 / "limitmust be less than or equal to 200"`。
const POOL_PAGE_LIMIT: u32 = 200;
/// 分页数上限（200 × 20 = 4000 行，远超任何真实交易日；实测单日最多 107 行）。
/// 收不满 `page.total` 时按错误处理，不出半池。
const POOL_MAX_PAGES: i64 = 20;
/// 接口 `field` 白名单：与既有 `get_hot_stocks` 同一份请求字段集（同一 URL，两种解全度）。
const POOL_FIELDS: &str =
    "199112,10,9001,330323,330324,330325,9002,330329,133971,133970,1968584,3475914,9003,9004";

/// 同花顺 JSON GET —— UA + Referer 是站点要求，429 单独判（否则限流被读成空数据）。
async fn ths_get_json(http: &reqwest::Client, url: &str) -> Result<Value, DataError> {
    let resp = http
        .get(url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .header("Referer", "https://data.10jqka.com.cn/")
        .send()
        .await?;
    crate::check_response_429(&resp, "ths")?;
    Ok(resp.json().await?)
}

/// `status_code` 闸门：非 0 一律失败，**不得**退化成空池。
///
/// 实测越界日期回 `-1 / "date参数不合法"`。若沿用 `get_hot_stocks` 那种不看状态码的写法，
/// 「日期超覆盖」与「那天确实一个涨停都没有」在下游就是同一个空数组。
fn pool_status_guard(json: &Value, date_compact: Option<&str>) -> Result<(), DataError> {
    let status = json.get("status_code").and_then(Value::as_i64);
    if status == Some(0) {
        return Ok(());
    }
    let msg = json.get("status_msg").and_then(Value::as_str).unwrap_or("无");
    Err(DataError::VendorError {
        vendor: "ths".into(),
        message: format!(
            "get_limit_up_pool 接口拒绝: status_code={status:?} msg={msg:?} date={date_compact:?}（超覆盖日期 ⇒ 该日不可得，不是当日无涨停）"
        ),
    })
}

/// 接口自报日期 `YYYYMMDD`（实测紧凑形）→ ISO `YYYY-MM-DD`。
///
/// 展开成 ISO 是 `is_for_requested_date()` 能成立的前提 —— 比对左值必须与调用方
/// 传入的 ISO 请求日同形，否则该判据恒假（P9-4 在 push2ex 版踩过一次未补零的同类坑）。
/// 拿不到自报日期就失败：没有它，这份池子属于哪一天无从判断。
fn normalize_pool_date(raw: Option<&str>) -> Result<String, DataError> {
    let d = raw.unwrap_or_default().replace('-', "");
    let b = d.as_bytes();
    if b.len() != 8 || !b.iter().all(|c| c.is_ascii_digit()) {
        return Err(DataError::ParseError(format!(
            "ths 涨停池未自报生效日期（data.date={raw:?}）⇒ 无法判定这份池子属于哪一天"
        )));
    }
    Ok(format!("{}-{}-{}", &d[0..4], &d[4..6], &d[6..8]))
}

/// Unix 秒 → `HH:MM:SS`（UTC+8）。
///
/// ⚠ 实测 `first_limit_up_time` 是**字符串**形式的秒级时间戳，不是东财那种 `HHMMSS` 整数。
fn cst_clock(v: Option<&Value>) -> Option<String> {
    let secs = match v? {
        Value::String(s) => s.parse::<i64>().ok()?,
        Value::Number(n) => n.as_i64()?,
        _ => return None,
    };
    let dt = chrono::DateTime::from_timestamp(secs, 0)?;
    let cst = chrono::FixedOffset::east_opt(8 * 3600).expect("UTC+8 偏移恒合法");
    Some(dt.with_timezone(&cst).format("%H:%M:%S").to_string())
}

/// `high_days_value` 位解码（实测 5 个交易日 320 行零反例）：**高 16 位 = 连板数，低 16 位 = 天数**。
///
/// 反例校核：「3天2板」= 131075 = (2 << 16) | 3 —— 与「天数在低位」一致；
/// 若按「天数在高位」读会得到 3 板 2 天，正好把弱连板读成强连板。
/// 解码不成立（板数 < 1 或 > 天数）时回 `None`，不编一个 1 进去。
fn decode_high_days(v: Option<i64>) -> (Option<i32>, Option<i32>) {
    let Some(v) = v else { return (None, None) };
    let streak = ((v >> 16) & 0xffff) as i32;
    let days = (v & 0xffff) as i32;
    if streak >= 1 && days >= streak {
        (Some(streak), Some(days))
    } else {
        (None, None)
    }
}

/// 一行涨停池。除 code/name 外逐字段 `Option`：实测 `open_num` 约 2/3 的行是 `null`。
fn parse_pool_row(item: &Value) -> Option<LimitUpPoolEntry> {
    let code = item.get("code").and_then(Value::as_str)?;
    let num = |k: &str| item.get(k).and_then(val_to_f64);
    let (limit_up_streak, limit_up_days) =
        decode_high_days(item.get("high_days_value").and_then(Value::as_i64));
    Some(LimitUpPoolEntry {
        stock_code: code.to_string(),
        stock_name: item.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
        limit_up_streak,
        limit_up_days,
        turnover_rate: num("turnover_rate"),
        change_pct: num("change_rate"),
        seal_amount: num("order_amount"),
        seal_volume: num("order_volume"),
        break_count: num("open_num").map(|v| v as i32),
        first_seal_time: cst_clock(item.get("first_limit_up_time")),
        last_seal_time: cst_clock(item.get("last_limit_up_time")),
        limit_up_type: item.get("limit_up_type").and_then(Value::as_str).map(str::to_string),
        re_sealed: item.get("is_again_limit").and_then(Value::as_i64).map(|v| v != 0),
        float_market_cap: num("currency_value"),
        latest_price: num("latest"),
        reason_tags: item
            .get("reason_type")
            .and_then(Value::as_str)
            .map(|s| {
                s.split('+').map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect()
            })
            .unwrap_or_default(),
    })
}

/// 盘面情绪汇总（`limit_up_count.today` + `limit_down_count.today`）。缺段则整体 `None`。
fn parse_limit_up_breadth(data: &serde_json::Map<String, Value>) -> Option<LimitUpBreadth> {
    let today = |key: &str| data.get(key).and_then(Value::as_object).and_then(|o| o.get("today"));
    let lu = today("limit_up_count")?.as_object()?;
    let ld = today("limit_down_count").and_then(Value::as_object);
    let i = |m: &serde_json::Map<String, Value>, k: &str| m.get(k).and_then(Value::as_i64);
    Some(LimitUpBreadth {
        limit_up_count: i(lu, "num")? as i32,
        touched_count: i(lu, "history_num").map(|v| v as i32),
        seal_rate: lu.get("rate").and_then(val_to_f64),
        break_count: i(lu, "open_num").map(|v| v as i32),
        limit_down_count: ld.and_then(|m| i(m, "num")).map(|v| v as i32),
    })
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod capability_tests {
    use super::*;

    fn make_vendor() -> ThsVendor {
        ThsVendor { http: reqwest::Client::new() }
    }

    #[test]
    fn ths_no_historical_methods() {
        let v = make_vendor();
        for m in &["get_hot_stocks", "get_industry_ranking", "get_concept_blocks"] {
            assert_eq!(v.asof_capability(m), AsOfCapability::NoHistoricalSemantic);
        }
    }

    #[test]
    fn ths_real_date_methods_are_fallthrough() {
        let v = make_vendor();
        for m in
            &["get_news", "get_market_dragon_tiger", "get_north_bound_flow", "get_consensus_eps"]
        {
            assert_eq!(v.asof_capability(m), AsOfCapability::Fallthrough);
        }
    }

    #[test]
    fn ths_stub_methods_are_fallthrough() {
        let v = make_vendor();
        for m in &[
            "get_quote",
            "get_klines",
            "get_financials",
            "get_money_flow",
            "get_dragon_tiger",
            "get_lockup_schedule",
            "search_stock",
        ] {
            assert_eq!(v.asof_capability(m), AsOfCapability::Fallthrough);
        }
    }
}

fn extract_json_between(text: &str, start: &str, end: &str) -> Option<String> {
    let start_idx = text.find(start)?;
    let json_start = start_idx + start.len();
    let json_end = text[json_start..].find(end)?;
    Some(text[json_start..json_start + json_end].to_string())
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod limit_up_pool_tests {
    use super::*;

    /// 实测响应切片（2026-10-03 打 `date=20260930`，五行原样；只删了 `time_preview` 分时数组）。
    ///
    /// 这五行覆盖了三个必须分开的形态：`open_num` 为 `null`（002041）、
    /// 炸板后回封（600059，`is_again_limit=1` + `open_num=4`）、集合竞价一字板（600825，09:25:00）。
    fn fixture() -> Value {
        serde_json::json!({
            "status_code": 0,
            "status_msg": "success",
            "data": {
                "date": "20260930",
                "page": {"limit": 200, "total": 5, "count": 1, "page": 1},
                "limit_up_count": {
                    "today": {"num": 52, "history_num": 64, "rate": 0.8125, "open_num": 12},
                    "yesterday": {"num": 56, "history_num": 64, "rate": 0.875, "open_num": 8}
                },
                "limit_down_count": {
                    "today": {"num": 8, "history_num": 15, "rate": 0.5333, "open_num": 7},
                    "yesterday": {"num": 10, "history_num": 22, "rate": 0.4545, "open_num": 12}
                },
                "info": [
                    {"open_num": null, "first_limit_up_time": "1790746113",
                     "last_limit_up_time": "1790746113", "code": "002041", "limit_up_type": "换手板",
                     "order_volume": 2954600, "is_new": 0, "limit_up_suc_rate": 1.0,
                     "currency_value": 9002400000.0, "market_id": 33, "is_again_limit": 0,
                     "change_rate": 10.0, "turnover_rate": 3.9203,
                     "reason_type": "玉米种业+转基因玉米+业绩增长", "order_amount": 30225558,
                     "high_days": "首板", "name": "登海种业", "high_days_value": 65537,
                     "change_tag": "FIRST_LIMIT", "market_type": "HS", "latest": 10.23},
                    {"open_num": 4, "first_limit_up_time": "1790731874",
                     "last_limit_up_time": "1790734733", "code": "600059", "limit_up_type": "换手板",
                     "order_volume": 5244699, "is_new": 0, "limit_up_suc_rate": 0.42857142857142855,
                     "currency_value": 11184625400.0, "market_id": 17, "is_again_limit": 1,
                     "change_rate": 10.0448, "turnover_rate": 9.8702,
                     "reason_type": "高端黄酒+全国化+年轻化", "order_amount": 64352457,
                     "high_days": "首板", "name": "古越龙山", "high_days_value": 65537,
                     "change_tag": "LIMIT_BACK", "market_type": "HS", "latest": 12.27},
                    {"open_num": 1, "first_limit_up_time": "1790733672",
                     "last_limit_up_time": "1790736597", "code": "002164", "limit_up_type": "换手板",
                     "order_volume": 4576720, "is_new": 0, "limit_up_suc_rate": 0.5714285714285714,
                     "currency_value": 6361818400.0, "market_id": 33, "is_again_limit": 1,
                     "change_rate": 10.0415, "turnover_rate": 10.4112,
                     "reason_type": "具身智能+精密减速器+股东增持", "order_amount": 60687307,
                     "high_days": "4天2板", "name": "宁波东力", "high_days_value": 131076,
                     "change_tag": "LIMIT_BACK", "market_type": "HS", "latest": 13.26},
                    {"open_num": null, "first_limit_up_time": "1790731500",
                     "last_limit_up_time": "1790731500", "code": "600825", "limit_up_type": "一字板",
                     "order_volume": 192114170, "is_new": 0, "limit_up_suc_rate": 0.75,
                     "currency_value": 10814589200.0, "market_id": 17, "is_again_limit": 0,
                     "change_rate": 9.9894, "turnover_rate": 1.3385,
                     "reason_type": "拟收购界面财联社+财经新媒体+上海国资", "order_amount": 1988381700,
                     "high_days": "7天7板", "name": "新华传媒", "high_days_value": 458759,
                     "change_tag": "FIRST_LIMIT", "market_type": "HS", "latest": 10.35},
                    {"open_num": 1, "first_limit_up_time": "1790732961",
                     "last_limit_up_time": "1790734824", "code": "002962", "limit_up_type": "换手板",
                     "order_volume": 3159584, "is_new": 0, "limit_up_suc_rate": 0.75,
                     "currency_value": 3383018400.0, "market_id": 33, "is_again_limit": 1,
                     "change_rate": 10, "turnover_rate": 17.0569,
                     "reason_type": "TGV光学+微棱镜+AR/VR", "order_amount": 51090473,
                     "high_days": "3天2板", "name": "五方光电", "high_days_value": 131075,
                     "change_tag": "LIMIT_BACK", "market_type": "HS", "latest": 16.17}
                ]
            }
        })
    }

    fn rows() -> Vec<Value> {
        fixture()["data"]["info"].as_array().unwrap().clone()
    }

    fn parse(code: &str) -> LimitUpPoolEntry {
        let row = rows()
            .into_iter()
            .find(|r| r["code"].as_str() == Some(code))
            .expect("fixture 内有该 code");
        parse_pool_row(&row).expect("有 code 的行都应解析成功")
    }

    /// 位编码方向是妖股判据的命门：`4天2板` 若按「天数在高 16 位」读就成了 4 连板。
    #[test]
    fn streak_is_high_bits_and_days_are_low_bits() {
        assert_eq!(
            (parse("002041").limit_up_streak, parse("002041").limit_up_days),
            (Some(1), Some(1))
        );
        assert_eq!(
            (parse("002164").limit_up_streak, parse("002164").limit_up_days),
            (Some(2), Some(4))
        );
        assert_eq!(
            (parse("002962").limit_up_streak, parse("002962").limit_up_days),
            (Some(2), Some(3))
        );
        assert_eq!(
            (parse("600825").limit_up_streak, parse("600825").limit_up_days),
            (Some(7), Some(7))
        );
    }

    /// 解码不成立（板数 < 1 或 天数 < 板数）时判「没有这个数」，不折成 1 或 0。
    #[test]
    fn impossible_bit_patterns_are_absent_not_fabricated() {
        assert_eq!(decode_high_days(Some((2 << 16) | 4)), (Some(2), Some(4)));
        assert_eq!(decode_high_days(Some(0x1_0000)), (None, None), "0 天不可能");
        assert_eq!(decode_high_days(Some((3 << 16) | 2)), (None, None), "3 板 2 天不可能");
        assert_eq!(decode_high_days(Some(0)), (None, None));
        assert_eq!(decode_high_days(None), (None, None));
    }

    /// `open_num=null` 与 `open_num=0` 不是一回事：前者是接口没说，后者是说过「没炸板」。
    #[test]
    fn null_break_count_stays_absent() {
        assert_eq!(parse("002041").break_count, None);
        assert_eq!(parse("002164").break_count, Some(1));
        assert_eq!(parse("600059").re_sealed, Some(true));
        assert_eq!(parse("002041").re_sealed, Some(false));
    }

    /// 实测时间是**字符串形式的 Unix 秒**，且按 UTC+8 才是交易时钟（09:25 是集合竞价一字板）。
    #[test]
    fn seal_times_are_unix_seconds_rendered_in_cst() {
        assert_eq!(parse("600825").first_seal_time.as_deref(), Some("09:25:00"));
        assert_eq!(parse("002041").first_seal_time.as_deref(), Some("13:28:33"));
        assert_eq!(parse("600059").last_seal_time.as_deref(), Some("10:18:53"));
        assert_eq!(cst_clock(Some(&serde_json::json!(""))), None, "空串不编造时间");
        assert_eq!(cst_clock(None), None);
    }

    /// 涨停逻辑的分隔符实测是 `+`，不是逗号 —— 按逗号拆会得到一整串标签。
    #[test]
    fn reason_tags_split_on_plus() {
        assert_eq!(
            parse("002041").reason_tags,
            vec!["玉米种业".to_string(), "转基因玉米".to_string(), "业绩增长".to_string()]
        );
    }

    #[test]
    fn amounts_turnover_and_float_cap_are_read_verbatim() {
        let e = parse("002041");
        assert_eq!(e.seal_amount, Some(30225558.0));
        assert_eq!(e.seal_volume, Some(2954600.0));
        assert_eq!(e.turnover_rate, Some(3.9203));
        assert_eq!(e.change_pct, Some(10.0));
        assert_eq!(e.float_market_cap, Some(9002400000.0));
        assert_eq!(e.latest_price, Some(10.23), "面板的现价列要用池内价，别再逐只打 quote");
        assert_eq!(e.limit_up_type.as_deref(), Some("换手板"));
    }

    #[test]
    fn pool_date_expands_to_iso_and_refuses_to_guess() {
        assert_eq!(normalize_pool_date(Some("20260930")).unwrap(), "2026-09-30");
        assert_eq!(normalize_pool_date(Some("20250918")).unwrap(), "2025-09-18");
        assert_eq!(normalize_pool_date(Some("2026-01-05")).unwrap(), "2026-01-05");
        for bad in [None, Some(""), Some("2025091"), Some("2025091x"), Some("2025年9月")] {
            assert!(normalize_pool_date(bad).is_err(), "{bad:?} 不该被接受");
        }
    }

    /// 主证：越界日期回 `status_code=-1`，必须成为**错误**而不是空池。
    /// 空池在下游会被读成「那天没有涨停」—— 这正是「拿不到」伪装成「没有」。
    #[test]
    fn rejected_date_is_error_not_empty_pool() {
        let rejected = serde_json::json!({"status_code": -1, "status_msg": "date参数不合法"});
        let err = pool_status_guard(&rejected, Some("20150105")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("不可得"), "错误文案要能自证是「不可得」，实得 {msg}");
        assert!(pool_status_guard(&fixture(), None).is_ok());
    }

    #[test]
    fn breadth_reads_today_segment_only() {
        let data = fixture()["data"].as_object().unwrap().clone();
        let b = parse_limit_up_breadth(&data).expect("实测带 limit_up_count");
        assert_eq!(b.limit_up_count, 52);
        assert_eq!(b.touched_count, Some(64));
        assert_eq!(b.seal_rate, Some(0.8125));
        assert_eq!(b.break_count, Some(12));
        assert_eq!(b.limit_down_count, Some(8));
        // 缺 limit_up_count ⇒ 整段缺席，不用 0 顶替
        let mut bare = data.clone();
        bare.remove("limit_up_count");
        assert!(parse_limit_up_breadth(&bare).is_none());
    }

    /// 判据要双向：只断言「不等时为假」会让恒假实现同样全绿（P9-4 在 push2ex 版踩过）。
    #[test]
    fn pool_belongs_to_requested_day_both_directions() {
        let mk = |pool_date: &str, requested: Option<&str>| LimitUpPool {
            pool_date: pool_date.into(),
            requested_date: requested.map(str::to_string),
            entries: vec![],
            breadth: None,
        };
        assert!(mk("2026-09-30", Some("2026-09-30")).is_for_requested_date());
        assert!(!mk("2026-09-30", Some("2026-09-25")).is_for_requested_date());
        assert!(mk("2026-09-30", None).is_for_requested_date());
        // 未补零的紧凑形与 ISO 请求日永不相等 ⇒ normalize 是判据成立的前提
        assert!(!mk("20260930", Some("2026-09-30")).is_for_requested_date());
    }

    #[test]
    fn limit_up_pool_is_declared_native_date_param() {
        let v = ThsVendor { http: reqwest::Client::new() };
        assert_eq!(v.asof_capability("get_limit_up_pool"), AsOfCapability::NativeDateParam);
        // 同一 URL 的 get_hot_stocks 仍是当下榜单 —— 两者语义不同，不可混用申报
        assert_eq!(v.asof_capability("get_hot_stocks"), AsOfCapability::NoHistoricalSemantic);
    }

    /// 真接口冒烟（`#[ignore]`，手工跑：
    /// `cargo test -p axagent-astock-data limit_up_pool_live -- --ignored --nocapture`）。
    ///
    /// 验三件实测才知道的事：`date` 是否仍被采纳并回显、覆盖左界是否仍在 2025-09 一带、
    /// 以及越界日期是否仍回错误而不是空池。
    #[tokio::test]
    #[ignore = "需真实网络；仅在手工验证涨停池通道时跑"]
    async fn limit_up_pool_live() {
        let v = ThsVendor { http: reqwest::Client::new() };

        let live = v.get_limit_up_pool(None).await.expect("当下涨停池应可达").expect("应有池子");
        assert!(!live.entries.is_empty(), "池子为空说明解析或头部有问题");
        assert!(live.is_for_requested_date());
        let streaks: Vec<i32> = live
            .entries
            .iter()
            .map(|e| e.limit_up_streak.expect("实测每行都带 high_days_value"))
            .collect();
        assert!(streaks.iter().any(|&s| s >= 2), "连板分布应含 >=2 的行，实得 {streaks:?}");
        let b = live.breadth.expect("实测带 limit_up_count");
        assert_eq!(b.limit_up_count as usize, live.entries.len(), "涨停家数应等于池子行数");
        println!(
            "当下涨停池 date={} 行数={} 最高连板={} 封板率={:?} 炸板={:?}",
            live.pool_date,
            live.entries.len(),
            streaks.iter().max().unwrap_or(&0),
            b.seal_rate,
            b.break_count
        );

        // 覆盖内的历史日：必须回显同日（这是本通道区别于已撤除的 push2ex 的地方）
        let back = v.get_limit_up_pool(Some("2025-09-18")).await.expect("覆盖内历史日应可达");
        let back = back.expect("有池子");
        assert_eq!(back.pool_date, "2025-09-18", "回显必须等于请求日");
        assert!(!back.entries.is_empty());
        println!("历史日 2025-09-18 行数={}", back.entries.len());

        // 越界：判「不可得」而不是空池
        let gone = v.get_limit_up_pool(Some("2015-01-05")).await;
        assert!(gone.is_err(), "超覆盖日期必须报错，实得 {gone:?}");
        println!("越界日期如实报错：{}", gone.unwrap_err());
    }
}

#[cfg(test)]
mod hot_list_tests {
    use super::*;

    /// 真·热股榜冒烟（`#[ignore]`，手工跑：
    /// `cargo test -p axagent-astock-data hot_list_live -- --ignored --nocapture`）。
    ///
    /// 锁三件实测才知道的事：
    /// 1. 端点仍是 `dq.10jqka.com.cn/fuyao/hot_list_data`（**不是**涨停池那个 URL —— 名目收编的落点）；
    /// 2. `rise_and_fall` 是**百分比原值**：既不能再乘 100（那会让新股 206.59% 变成 20659），
    ///    也不是小数比例（那样全榜 |值| 都会 < 0.11）；
    /// 3. `order` 是 1 起的名次且严格递增（名次被写乱时这里先红）。
    #[tokio::test]
    #[ignore = "需真实网络；仅在手工验证热股榜通道时跑"]
    async fn hot_list_live() {
        let v = ThsVendor { http: reqwest::Client::new() };
        let list = v.get_hot_stocks().await.expect("热股榜应可达");
        assert!(!list.is_empty(), "实测恒 100 行，空即接口异常");
        assert_eq!(list[0].rank, Some(1), "首行名次应为 1");
        let mut prev = 0u32;
        for h in &list {
            let r = h.rank.expect("实测每行都带 order");
            assert!(r > prev, "名次必须严格递增（容忍 filter_map 跳行），实得 {prev} -> {r}");
            prev = r;
        }
        assert!(list.iter().all(|h| h.hot_value.is_some()), "热度值实测每行都有");
        let max_abs = list.iter().map(|h| h.change_pct.abs()).fold(0.0f64, f64::max);
        let over_one = list.iter().filter(|h| h.change_pct.abs() > 1.0).count();
        assert!(max_abs <= 500.0, "若涨幅被再乘 100，新股首日会出现上万量级，实得 {max_abs}");
        assert!(over_one > 0, "全榜 |涨幅| 都 < 1 说明把百分比当成了小数比例");
        println!(
            "热股榜 {} 行 榜首={}/{} 涨幅={:.2}% 热度={:?} 题材={:?}",
            list.len(),
            list[0].stock_code,
            list[0].stock_name,
            list[0].change_pct,
            list[0].hot_value,
            list[0].reason_tags
        );
    }
}
