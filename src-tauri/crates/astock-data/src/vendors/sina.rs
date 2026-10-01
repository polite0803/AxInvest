use crate::as_of_capability::AsOfCapability;
use crate::error::DataError;
use crate::types::*;
use crate::vendors::eastmoney::{money_flow_from_window, select_fflow_window};
use crate::vendors::StockVendor;
use async_trait::async_trait;

pub struct SinaVendor {
    pub http: reqwest::Client,
}

impl SinaVendor {
    /// 带 429 检测的 GET 请求
    async fn sina_get(&self, url: &str) -> Result<reqwest::Response, DataError> {
        let resp =
            self.http.get(url).header("Referer", "https://finance.sina.com.cn/").send().await?;
        crate::check_response_429(&resp, "sina")?;
        Ok(resp)
    }

    /// 备选新闻端点：尝试其他已知的新浪新闻接口
    async fn get_news_fallback(
        &self,
        stock_code: &str,
        limit: u32,
    ) -> Result<Vec<NewsItem>, DataError> {
        let url = format!(
            "https://vip.stock.finance.sina.com.cn/quotes_service/api/json_v2.php/Market_Center.getStockNews?code={stock_code}&num={}&page=1&type=last",
            limit.min(50)
        );
        let resp = self.sina_get(&url).await?;

        // 修复 P0-A5: 原 `unwrap_or_default()` 把 HTTP body 解码错误吞为空串，
        // 走到下面 `is_empty()` 分支报"返回空响应"丢失根因。
        // 改用 `?` 透传原始 reqwest::Error 便于调试。
        let body = resp.text().await?;
        let trimmed = body.trim();

        // 检查空响应
        if trimmed.is_empty() {
            return Err(DataError::VendorError {
                vendor: "sina".into(),
                message: "新浪新闻备选端点返回空响应".into(),
            });
        }

        // 检查 JSONP 包裹
        let json_str = if let Some(start) = trimmed.find('(') {
            if let Some(end) = trimmed.rfind(')') {
                if end > start {
                    &trimmed[start + 1..end]
                } else {
                    trimmed
                }
            } else {
                trimmed
            }
        } else {
            trimmed
        };

        // 检查错误响应
        if json_str.contains("__ERROR") || json_str.contains("Service not found") {
            return Err(DataError::VendorError {
                vendor: "sina".into(),
                message: format!("新浪新闻备选端点不可用: {json_str:.100}"),
            });
        }

        let json: serde_json::Value = serde_json::from_str(json_str).map_err(|e| {
            DataError::ParseError(format!(
                "sina fallback news parse: {e}, raw={}",
                &trimmed[..trimmed.len().min(120)]
            ))
        })?;

        let items = json
            .as_array()
            .or_else(|| json["result"].as_array())
            .or_else(|| json["data"].as_array())
            .cloned()
            .unwrap_or_default();

        Ok(items
            .iter()
            .map(|item| NewsItem {
                title: item["title"].as_str().unwrap_or("").to_string(),
                summary: item["summary"]
                    .as_str()
                    .or_else(|| item["digest"].as_str())
                    .unwrap_or("")
                    .to_string(),
                source: item["source"].as_str().unwrap_or("新浪财经").to_string(),
                url: item["url"]
                    .as_str()
                    .or_else(|| item["article_url"].as_str())
                    .unwrap_or("")
                    .to_string(),
                publish_time: item["ctime"]
                    .as_str()
                    .or_else(|| item["date"].as_str())
                    .unwrap_or("")
                    .to_string(),
                sentiment_score: None,
            })
            .collect())
    }

    /// 备选新闻端点失败后，降级到东方财富搜索 API
    async fn get_news_fallback_or_eastmoney(
        &self,
        stock_code: &str,
        limit: u32,
    ) -> Result<Vec<NewsItem>, DataError> {
        match self.get_news_fallback(stock_code, limit).await {
            Ok(news) => Ok(news),
            Err(e) => {
                tracing::warn!("[sina] 新闻备选端点也失败，降级到东方财富: {e}");
                crate::vendors::fetch_eastmoney_news(&self.http, "sina", stock_code, limit).await
            },
        }
    }

    /// 新浪 K 线通道（分钟级；日线继续走网易通道，未做切换）。
    ///
    /// 口径说明：
    /// - `volume` 实测单位已是**股**（002041 / 688806 / 600519 三个市场的量级都对得上），
    ///   不再做 ×100 换算（网易通道那份是「手」，两处口径不同，各自注释标明）。
    /// - 接口**不返回成交额** ⇒ `amount = 0.0`。不拿 `volume * close` 估算：那是编造值，
    ///   会污染下游一切「额」类指标。
    /// - 不复权（`ma=no`）⇒ `adj_factor = None`，由 lib 层按 adj_type 本地复权。
    async fn get_klines_from_sina(
        &self,
        stock_code: &str,
        period: &str,
        limit: u32,
    ) -> Result<Vec<KLine>, DataError> {
        let url = match sina_kline_url(stock_code, period, limit) {
            Some(u) => u,
            // 未映射的周期（如年线）：维持原「空」行为，交上层继续 fallback
            None => return Ok(vec![]),
        };
        let resp = self.sina_get(&url).await?;
        let json: serde_json::Value = resp.json().await.map_err(|e| DataError::VendorError {
            vendor: "sina".into(),
            message: format!("新浪K线 JSON 解析失败: {e}"),
        })?;
        let items = match json.as_array() {
            Some(arr) => arr,
            // 非数组 ⇒ 接口形态变了（或命中风控页）。报错而不是返空，否则会退化成
            // 「静默无K线」—— 正是本次修复要防的那种失效。
            None => {
                return Err(DataError::VendorError {
                    vendor: "sina".into(),
                    message: "新浪K线返回非数组（接口形态变更或风控拦截）".into(),
                });
            },
        };

        Ok(items
            .iter()
            .filter_map(|item| {
                let date = item["day"].as_str()?;
                if date.is_empty() {
                    return None;
                }
                // match 而非长链：链宽留在 chain_width(60) 内，避免 fmt --check 拆行
                let f = |key: &str| -> f64 {
                    match &item[key] {
                        serde_json::Value::String(s) => s.parse().unwrap_or(0.0),
                        other => other.as_f64().unwrap_or(0.0),
                    }
                };
                Some(KLine {
                    date: date.to_string(),
                    open: f("open"),
                    high: f("high"),
                    low: f("low"),
                    close: f("close"),
                    volume: f("volume"),
                    amount: 0.0,
                    turnover_rate: None,
                    adj_factor: None,
                })
            })
            .collect())
    }
}

/// `sh600519` / `sz000001` —— live 与 as-of 共用，避免两份前缀实现漂移
fn sina_daima(stock_code: &str) -> String {
    let code = stock_code.trim();
    let market = if code.starts_with('6') || code.starts_with('9') {
        "sh"
    } else if code.starts_with('4') || code.starts_with('8') {
        // 北交所（4/8 开头）实测 `bj430047` 可用；此前一律落进 `sz` ⇒ 请求恒空 ——
        // 而且新浪对不存在的 symbol 返回的是**空数组**而非错误，静默无声。
        "bj"
    } else {
        "sz"
    };
    format!("{market}{code}")
}

/// 新浪 K 线 URL（`CN_MarketData.getKLineData`）—— **分钟级通道**。
///
/// 为什么需要它（2026-10-01 修正）：原实现注释称「新浪无直接K线接口，用163补」，
/// 并据此把**所有非日线周期直接返空**（`Ok(vec![])`）。该前提不成立 —— 实测该接口
/// 原生支持 scale=5/15/30/60 分钟与 240 日线，且是当时**唯一**同时满足「支持 m60」
/// 与「可达」的源：2026-10-01 运行日志里 002041 / 688806 的 m60 请求正是
/// push2his（连接被掐断）、web.ifzq.gtimg.cn（连不上）、TDX 7709（超时）三源全灭。
///
/// `daily → 240` 现已启用：`get_klines` 的日线也走本函数。原先日线走网易 chddata，
/// 但该接口 2026-10-01 实测 `502 Bad Gateway`、运行日志里日线亦恒空（见 `get_klines`
/// 的注释），故日线统一到这条已实测可用的通道。
///
/// 周/月线现已映射：`1200`=周线、`7200`=月线（实测依据见下方 match 内注释）。
/// 此前两者都缺失，使周/月线在整条路由上无源可依。
fn sina_kline_url(stock_code: &str, period: &str, limit: u32) -> Option<String> {
    let scale = match period {
        "5" | "Min5" => 5,
        "15" | "Min15" => 15,
        "30" | "Min30" => 30,
        "60" | "Min60" => 60,
        "daily" | "101" | "Daily" => 240,
        // 周/月线：实测 `scale=1200` 返回周线（每交易日一个采样）、`7200` 返回月线
        // （2026-07-31 / 08-31 / 09-30，每月末一条）。此前不映射 ⇒ 周/月线**全链无源**
        // （2026-10-01 日志：002164 的 `klt=103` 在 tencent/eastmoney/xueqiu/sina/mootdx
        // 五个源上全空）。未映射的周期仍返 None，不猜。
        "weekly" | "102" | "Weekly" => 1200,
        "monthly" | "103" | "Monthly" => 7200,
        _ => return None,
    };
    // datalen 保守上限 1000：接口对超大 datalen 的行为未实测，超出部分由
    // 「拉到多少算多少」兜住，不值得为多要几条去冒被拒的风险。
    let datalen = limit.min(1000);
    let daima = sina_daima(stock_code);
    Some(format!(
        "https://money.finance.sina.com.cn/quotes_service/api/json_v2.php/\
        CN_MarketData.getKLineData?symbol={daima}&scale={scale}&ma=no&datalen={datalen}"
    ))
}

/// 一次取回的历史天数（裁到截止日后取最近 5 条，与 eastmoney 同口径）
const SINA_FFLOW_HISTORY_ROWS: u32 = 20;

/// 新浪个股资金流**按日历史**接口（实测 2026-09-27：`asc=0` 即按 opendate 降序）
fn sina_fflow_history_url(daima: &str, num: u32) -> String {
    format!(
        "https://vip.stock.finance.sina.com.cn/quotes_service/api/json_v2.php/        MoneyFlow.ssl_qsfx_zjlrqs?page=1&num={num}&sort=opendate&asc=0&daima={daima}"
    )
}

/// 该表只有主力（netamount）与超大单（r0_net）两档 ⇒ 其余三档 `None`，不是 0
fn parse_sina_fflow_rows(rows: &[serde_json::Value]) -> Vec<MoneyFlowDaily> {
    let num = |r: &serde_json::Value, k: &str| -> Option<f64> {
        r.get(k).and_then(|v| match v.as_str() {
            Some(s) => s.parse::<f64>().ok(),
            None => v.as_f64(),
        })
    };
    rows.iter()
        .filter_map(|r| {
            let date = r["opendate"].as_str().unwrap_or("").to_string();
            if date.is_empty() {
                return None;
            }
            // 主力净流入缺失 ⇒ 这一行没有可陈述的事实，整行丢弃（不补 0）
            let main = num(r, "netamount")?;
            Some(MoneyFlowDaily {
                date,
                main_net_inflow: main,
                super_large_net: num(r, "r0_net"),
                large_net: None,
                medium_net: None,
                small_net: None,
            })
        })
        .collect()
}

#[async_trait]
impl StockVendor for SinaVendor {
    async fn get_quote(&self, stock_code: &str) -> Result<StockQuote, DataError> {
        let prefix = if stock_code.starts_with('6') {
            "sh"
        } else if stock_code.starts_with('8') || stock_code.starts_with('4') {
            "bj"
        } else {
            "sz"
        };
        let url = format!("https://hq.sinajs.cn/list={prefix}{stock_code}");
        let resp = self.sina_get(&url).await?;
        let bytes = resp.bytes().await?;
        // 新浪财经 API 使用 GBK 编码
        let body = encoding_rs::GBK.decode(&bytes).0;
        // 格式: var hq_str_sz000001="平安银行,12.50,12.30,12.60,12.80,..."
        let start = body
            .find('"')
            .ok_or_else(|| DataError::ParseError("sina quote parse: no opening quote".into()))?;
        let end = body[start + 1..]
            .find('"')
            .ok_or_else(|| DataError::ParseError("sina quote parse: no closing quote".into()))?;
        let data = &body[start + 1..start + 1 + end];
        let fields: Vec<&str> = data.split(',').collect();
        if fields.len() < 32 {
            return Err(DataError::ParseError(format!(
                "sina quote: expected >=32 fields, got {}",
                fields.len()
            )));
        }
        let f = |i: usize| -> f64 { fields.get(i).and_then(|s| s.parse().ok()).unwrap_or(0.0) };
        Ok(StockQuote {
            code: stock_code.to_string(),
            name: fields.first().copied().unwrap_or("").to_string(),
            price: f(3),
            pre_close: f(2),
            open: f(1),
            high: f(4),
            low: f(5),
            // H4 实测回退（2026-08-11 实网探测, sh600519）:
            //   sina f(8)=6268572、f(9)=8428304269，且 1348.86 × f(8) ≈ f(9)
            //   → f(8) 本身已是「股」、f(9) 本身已是「元」，无需换算。
            //   7-13 的 ×100/×10000 是错误修复（把 volume 放大 100 倍、amount 放大 1e4 倍），
            //   会污染 quote 缓存与下游指标，故回退直取。
            volume: f(8),
            amount: f(9),
            change_pct: (f(3) - f(2)) / f(2) * 100.0,
            turnover_rate: 0.0,
            pe: None,
            pb: None,
            total_mv: None,
            circulating_mv: None,
            limit_up: None,
            limit_down: None,
            is_st: false,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })
    }

    async fn get_klines(
        &self,
        stock_code: &str,
        period: &str,
        limit: u32,
        _adj: Option<AdjType>,
    ) -> Result<Vec<KLine>, DataError> {
        // 日线与分钟级统一走新浪 `CN_MarketData.getKLineData`。
        //
        // 依据（2026-10-01 运行日志 + 独立实测）：原实现让日线走网易 chddata，而该接口
        // 已实测返回 `502 Bad Gateway`，body 里没有可用 CSV ⇒ 解析出 0 条 ⇒ 上层判
        // 「返回空数据(非故障)」，**日线静默全空且不降级**（日志里
        // `klines 000001 sina 失败: K线返回空` 走的正是这条已死的网易路径）。
        // 新浪日线（scale=240）实测对沪深与科创板均可用（002041 / 688806 / 600519），
        // 与分钟级同源，故不再维护两个通道 —— 网易那段实现连同其 code 与字段口径一并
        // 移除（需要恢复时见 git 历史：code=市场位(0沪/1深)+代码，字段顺序
        // date,TCLOSE,HIGH,LOW,TOPEN,LCLOSE,VOTURNOVER(手),VATURNOVER(元)）。
        self.get_klines_from_sina(stock_code, period, limit).await
    }

    async fn get_financials(&self, stock_code: &str) -> Result<Vec<FinancialReport>, DataError> {
        // 原实现走网易 `quotes.money.163.com/service/zycwzb_*`（财务指标 CSV）。
        // 2026-10-01 实测该域名恒返回 `502 Bad Gateway`（与其 kline 的 chddata 同域同时
        // 失效），通道整体不可用 ⇒ 整段删除，而不是留着"能连上但解析出 0 条"的版本 ——
        // 后者会伪装成「该股没有财报」，正是本轮系列修复要消除的失效形态。
        //
        // 另注：本 vendor **不在** `financials` 路由表内（见 `lib.rs` 的 `default_routing`），
        // 所以此处改动当前不产生行为变化；保留一个**显式 Err** 是为了将来若有人把它加回
        // 路由时，第一次调用就能看到明确原因，而不是静默返空。
        // 要真正启用，须先接一条实测可用的源（新浪的财务页是 HTML，解析成本另计）。
        let _ = stock_code;
        Err(DataError::VendorError {
            vendor: "sina".into(),
            message: "sina 财务通道已下线（网易 zycwzb 502）".into(),
        })
    }

    async fn get_news(&self, stock_code: &str, limit: u32) -> Result<Vec<NewsItem>, DataError> {
        // 降级链路：新浪主端点 → 新浪备选端点 → 东方财富
        let primary_url = format!(
            "https://vip.stock.finance.sina.com.cn/corp/go.php/vCB_AllNewsStock/symbol/{stock_code}.json?page=1&num={}",
            limit.min(50)
        );

        let resp = match self.sina_get(&primary_url).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("[sina] 新闻主端点请求失败，尝试备选端点: {e}");
                return self.get_news_fallback_or_eastmoney(stock_code, limit).await;
            },
        };

        let ct = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok().map(String::from))
            .unwrap_or_default();

        // 如果 Content-Type 不是 JSON，放弃解析并尝试备选端点
        if !ct.contains("json") && !ct.contains("javascript") {
            let body_len = resp.content_length().unwrap_or(0);
            tracing::warn!(
                "[sina] 新闻主端点返回非 JSON (Content-Type={ct}, Content-Length={body_len})，尝试备选端点"
            );
            return self.get_news_fallback_or_eastmoney(stock_code, limit).await;
        }

        let items: Vec<serde_json::Value> =
            resp.json().await.map_err(|e| DataError::VendorError {
                vendor: "sina".into(),
                message: format!("新闻 JSON 解析失败: {e} (Content-Type={ct}, url={primary_url})"),
            })?;

        if items.is_empty() {
            tracing::warn!("[sina] 新闻主端点返回空数组，尝试备选端点");
            return self.get_news_fallback_or_eastmoney(stock_code, limit).await;
        }

        Ok(items
            .iter()
            .map(|item| NewsItem {
                title: item["title"].as_str().unwrap_or("").to_string(),
                summary: String::new(),
                source: "新浪财经".to_string(),
                url: format!("https://finance.sina.com.cn{}", item["url"].as_str().unwrap_or("")),
                publish_time: item["ctime"].as_str().unwrap_or("").to_string(),
                sentiment_score: None,
            })
            .collect())
    }

    async fn get_money_flow(&self, stock_code: &str) -> Result<Option<MoneyFlow>, DataError> {
        // 新浪财经资金流向 API（个股，当日单点）
        let daima = sina_daima(stock_code);
        let url = format!(
            "https://vip.stock.finance.sina.com.cn/quotes_service/api/json_v2.php/MoneyFlow.ssi_ssfx_flzjtj?format=text&daima={daima}"
        );
        let resp = self.sina_get(&url).await?;
        let json: serde_json::Value = resp.json().await?;

        let parse = |key: &str| -> f64 {
            json.get(key).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0)
        };

        let r0_in = parse("r0_in");
        let r0_out = parse("r0_out");
        let r1_in = parse("r1_in");
        let r1_out = parse("r1_out");
        let r2_in = parse("r2_in");
        let r2_out = parse("r2_out");
        let r3_in = parse("r3_in");
        let r3_out = parse("r3_out");

        // 如果所有字段都是 0，说明请求失败或股票无数据
        if r0_in == 0.0 && r0_out == 0.0 && r1_in == 0.0 && r1_out == 0.0 {
            return Ok(None);
        }

        // R1-修复: 新浪 API 未返回交易日期，用当前日期（UTC+8 北京时间）作为兜底。
        //   原 Local::now() 在非中国时区部署时会偏移一天。
        let today = chrono::Utc::now()
            .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
            .format("%Y-%m-%d")
            .to_string();

        Ok(Some(MoneyFlow {
            date: today,
            // 主力净流入 = 超大单净流入 + 大单净流入
            main_net_inflow: (r0_in - r0_out) + (r1_in - r1_out),
            super_large_net: Some(r0_in - r0_out),
            large_net: Some(r1_in - r1_out),
            medium_net: Some(r2_in - r2_out),
            small_net: Some(r3_in - r3_out),
            history: Vec::new(),
        }))
    }

    /// T17(2026-09-27)：回放里的资金流。
    ///
    /// 为什么换源：日频资金流在 `push2his.eastmoney.com/fflow/daykline`，而本机对该域是
    /// **连接级拒绝**（2026-08-01、2026-09-27 两次实测；真 Edge + `--disable-ipv6` 也回
    /// `net::ERR_EMPTY_RESPONSE`）⇒ 直连与内核兜底两条路一起死，`browser_eastmoney` 绕的是
    /// TLS 指纹与 CORS，绕不了主机不回包。新浪这张表在没被墙的域上，且**接口自带日期序列**
    /// （`sort=opendate&asc=0` 实测降序）⇒ 取回后按截止日裁，形态与 eastmoney 完全一致。
    ///
    /// 只披露主力与超大单两档：大/中/小单如实 `None`（见 `MoneyFlow` 字段的 None≠0 约定），
    /// 不为凑齐五档写 0。
    async fn get_money_flow_with_asof(
        &self,
        stock_code: &str,
    ) -> Result<Option<MoneyFlow>, DataError> {
        let as_of = crate::as_of::current_as_of()
            .ok_or_else(|| DataError::ParseError("no as_of context".into()))?;
        let cutoff = as_of.as_of_date.format("%Y-%m-%d").to_string();
        let url = sina_fflow_history_url(&sina_daima(stock_code), SINA_FFLOW_HISTORY_ROWS);
        let resp = self.sina_get(&url).await?;
        let json: serde_json::Value = resp.json().await?;
        let rows = match json.as_array() {
            Some(arr) if !arr.is_empty() => arr,
            _ => return Ok(None),
        };
        Ok(money_flow_from_window(select_fflow_window(parse_sina_fflow_rows(rows), &cutoff)))
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

    // ── P3:sina 能力申报 ──
    fn asof_capability(&self, method: &str) -> AsOfCapability {
        match method {
            "get_quote" => AsOfCapability::SynthesizeFromKline,
            // T17(2026-09-27)：`MoneyFlow.ssl_qsfx_zjlrqs` 是按日历史（实测 200，
            // 字段含 opendate / netamount / r0_net），且不落在 push2his 上 ⇒ 回放有真通道
            "get_money_flow" => AsOfCapability::NativeDateParam,
            _ => AsOfCapability::Fallthrough,
        }
    }
}

#[cfg(test)]
mod capability_tests {
    use super::*;

    fn make_vendor() -> SinaVendor {
        SinaVendor { http: reqwest::Client::new() }
    }

    #[test]
    fn sina_quote_is_synthesize() {
        let v = make_vendor();
        assert_eq!(v.asof_capability("get_quote"), AsOfCapability::SynthesizeFromKline);
    }

    #[test]
    fn sina_others_are_fallthrough() {
        let v = make_vendor();
        for m in &[
            "get_news",
            "get_klines",
            "get_financials",
            "get_dragon_tiger",
            "get_lockup_schedule",
            "search_stock",
        ] {
            assert_eq!(v.asof_capability(m), AsOfCapability::Fallthrough);
        }
    }

    /// T17：资金流在 sina 有按日历史通道 ⇒ 必须申报 NativeDateParam，
    /// 否则路由层的 as-of 白名单会跳过这个**唯一没被墙**的资金流源。
    #[test]
    fn sina_money_flow_is_native_date_param() {
        let v = make_vendor();
        assert_eq!(v.asof_capability("get_money_flow"), AsOfCapability::NativeDateParam);
    }
}

#[cfg(test)]
mod sina_fflow_asof_tests {
    use super::*;

    /// 2026-09-27 实测响应（6 条，opendate 降序）裁剪后的样本
    fn fixture() -> Vec<serde_json::Value> {
        ["2026-09-24", "2026-09-23", "2026-09-22", "2026-09-21"]
            .iter()
            .enumerate()
            .map(|(i, d)| {
                serde_json::json!({
                    "opendate": d,
                    "trade": "1238.0000",
                    "changeratio": "-0.0105815",
                    "netamount": format!("-{}.0", (i + 1) * 100),
                    "r0_net": format!("-{}.0", (i + 1) * 90),
                    "cate_na": "-1501098460.4200"
                })
            })
            .collect()
    }

    #[test]
    fn url_carries_daima_and_descending_date_sort() {
        let url = sina_fflow_history_url(&sina_daima("600519"), 20);
        assert!(url.contains("daima=sh600519"), "{url}");
        assert!(url.contains("sort=opendate&asc=0"), "日期降序是裁窗前提: {url}");
        assert!(url.contains("num=20"), "{url}");
    }

    #[test]
    fn unreported_tiers_stay_none_instead_of_zero() {
        let rows = parse_sina_fflow_rows(&fixture());
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].date, "2026-09-24");
        assert_eq!(rows[0].main_net_inflow, -100.0);
        assert_eq!(rows[0].super_large_net, Some(-90.0));
        // 该表不披露大/中/小单 ⇒ None；写成 0.0 会被下游读成「这一档净额为零」
        assert_eq!(rows[0].large_net, None);
        assert_eq!(rows[0].medium_net, None);
        assert_eq!(rows[0].small_net, None);
    }

    #[test]
    fn missing_main_amount_drops_the_row() {
        let rows = parse_sina_fflow_rows(&[
            serde_json::json!({ "opendate": "2026-09-24", "r0_net": "-900.0" }),
            serde_json::json!({ "opendate": "2026-09-23", "netamount": "-800.0" }),
        ]);
        assert_eq!(rows.len(), 1, "主力净流入缺失的行不补 0，整行丢弃");
        assert_eq!(rows[0].date, "2026-09-23");
    }

    /// 与 eastmoney 共用裁窗 + 装配 ⇒ 两条源给出的是同一形态、同一口径
    #[test]
    fn asof_window_truncates_to_cutoff_and_keeps_shape() {
        let mf = money_flow_from_window(select_fflow_window(
            parse_sina_fflow_rows(&fixture()),
            "2026-09-22",
        ))
        .expect("截止日在窗口内 ⇒ 应有值");
        assert_eq!(mf.date, "2026-09-22", "顶层必须锚在截止日当天（有披露时）");
        assert!(mf.history.iter().all(|h| h.date.as_str() <= "2026-09-22"), "{:?}", mf.history);
        assert_eq!(mf.large_net, None);
        assert!(mf.history.iter().all(|h| h.large_net.is_none()));
    }
}

#[cfg(test)]
mod sina_kline_tests {
    //! 2026-10-01：新浪 K 线（分钟级）通道的判据（零网络）。
    //!
    //! 缺陷背景：原实现认为「新浪无直接K线接口」，把所有非日线周期直接返空，
    //! 于是 m60 在 push2his / 腾讯 / TDX 三源全灭时无源可用。这里钉住
    //! ①分钟周期真的生成新浪 URL（而不是又一次返空）②日线映射到 scale=240
    //! ③datalen 有保守上限。

    use super::*;

    #[test]
    fn minute_periods_map_to_sina_scale() {
        let u = sina_kline_url("sz002041", "60", 500).expect("m60 必须有新浪通道");
        assert!(u.contains("symbol=sz002041"), "{u}");
        assert!(u.contains("scale=60"), "60 分钟必须映射到 scale=60: {u}");
        assert!(u.contains("ma=no"), "不复权交由 lib 层本地应用: {u}");
        // ⚠ 2026-10-01 补：`sina_kline_url` 新增第 3 参 `limit` 时，本处是**唯一**漏改的
        //   调用点（同模块其余调用点都已带实参）⇒ 整个 crate 的测试目标编译失败。
        //   取值与上一行同用例的 60 分钟分支一致（`500`）；本断言只看 `scale=5`。
        let m5 = sina_kline_url("600519", "Min5", 500).expect("Min5 必须有通道");
        assert!(m5.contains("symbol=sh600519"), "{m5}");
        assert!(m5.contains("scale=5"), "{m5}");
    }

    /// 日线 → 240；周线 → 1200；月线 → 7200。周/月线此前缺映射 ⇒ 全链无源
    /// （2026-10-01 日志：002164 的 `klt=103` 在五个源上全空）
    #[test]
    fn daily_weekly_monthly_map_to_sina_scale() {
        let d = sina_kline_url("600519", "daily", 100).expect("daily");
        assert!(d.contains("scale=240"), "{d}");
        let w = sina_kline_url("600519", "weekly", 100).expect("weekly");
        assert!(w.contains("scale=1200"), "{w}");
        assert!(sina_kline_url("600519", "102", 100).is_some(), "102 是周线");
        let m = sina_kline_url("600519", "103", 100).expect("103 是月线");
        assert!(m.contains("scale=7200"), "{m}");
        // 未映射的周期仍返 None：不猜周期，让上层继续 fallback
        assert!(sina_kline_url("600519", "yearly", 100).is_none());
    }

    /// 北交所（4/8 开头）必须走 `bj` 前缀：实测 `bj430047` 可用，
    /// 而原先一律落进 `sz` ⇒ 请求恒空（新浪对不存在的 symbol 返回空数组，不报错）
    #[test]
    fn beijing_exchange_uses_bj_prefix() {
        let d = sina_kline_url("430047", "daily", 100).expect("daily");
        assert!(d.contains("symbol=bj430047"), "{d}");
        let m = sina_kline_url("833171", "60", 100).expect("m60");
        assert!(m.contains("symbol=bj833171"), "{m}");
    }

    /// datalen 有保守上限：接口对超大值的容忍度未实测，不能被一个超大 limit 带崩
    #[test]
    fn datalen_is_capped() {
        let u = sina_kline_url("600519", "60", 9999).expect("m60");
        assert!(u.contains("datalen=1000"), "应被压到保守上限: {u}");
    }
}
