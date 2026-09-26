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
}

/// `sh600519` / `sz000001` —— live 与 as-of 共用，避免两份前缀实现漂移
fn sina_daima(stock_code: &str) -> String {
    let code = stock_code.trim();
    let market = if code.starts_with('6') || code.starts_with('9') {
        "sh"
    } else {
        "sz"
    };
    format!("{market}{code}")
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
        // 网易163 chddata API 仅支持日K线；分钟/周/月线由高优先级 vendor 承担，
        // 非日K周期直接返回空，避免返回错误周期的数据
        if !matches!(period, "daily" | "101" | "Daily") {
            return Ok(vec![]);
        }
        // 网易K线API code格式：0+沪市代码, 1+深市代码（与财务API的sh/sz前缀不同）
        let market = if stock_code.starts_with('6') || stock_code.starts_with('9') {
            "0"
        } else {
            "1"
        };
        // 网易财经历史日K线API（新浪无直接K线接口，用163补）
        // fields顺序: date, TCLOSE(收盘), HIGH(最高), LOW(最低), TOPEN(开盘), LCLOSE(昨收), VOTURNOVER(成交量,手), VATURNOVER(成交额,元)
        let url = format!(
            "https://quotes.money.163.com/service/chddata.html?code={market}{stock_code}&start=20200101&end=20500101&fields=TCLOSE;HIGH;LOW;TOPEN;LCLOSE;VOTURNOVER;VATURNOVER"
        );
        let resp = self.http.get(&url).header("Referer", "https://money.163.com/").send().await?;
        crate::check_response_429(&resp, "sina")?;
        let body = resp.text().await?;
        let mut klines = Vec::new();
        for line in body.lines().skip(1) {
            let fields: Vec<&str> = line.split(',').collect();
            if fields.len() < 8 {
                continue;
            }
            let f = |i: usize| -> f64 {
                fields.get(i).and_then(|s| s.trim().parse().ok()).unwrap_or(0.0)
            };
            let date = fields[0].trim().trim_matches('\'').to_string();
            if date.is_empty() {
                continue;
            }
            klines.push(KLine {
                date,
                open: f(4),           // TOPEN(开盘)
                high: f(2),           // HIGH(最高)
                low: f(3),            // LOW(最低)
                close: f(1),          // TCLOSE(收盘)
                volume: f(6) * 100.0, // VOTURNOVER(手) → 股
                amount: f(7),         // VATURNOVER(元)
                turnover_rate: None,
                adj_factor: None, // 网易不支持复权
            });
        }
        klines.sort_by(|a, b| a.date.cmp(&b.date));
        if klines.len() > limit as usize {
            let start = klines.len() - limit as usize;
            klines = klines[start..].to_vec();
        }
        Ok(klines)
    }

    async fn get_financials(&self, stock_code: &str) -> Result<Vec<FinancialReport>, DataError> {
        let market = if stock_code.starts_with('6') {
            "sh"
        } else {
            "sz"
        };
        // 网易财经财务指标 API
        let url = format!(
            "https://quotes.money.163.com/service/zycwzb_{market}{stock_code}.html?type=report&start=2020&end=2026"
        );
        let resp = self.http.get(&url).header("Referer", "https://money.163.com/").send().await?;
        crate::check_response_429(&resp, "sina")?;
        let body = resp.text().await?;
        let mut reports = Vec::new();
        for line in body.lines().skip(2) {
            let fields: Vec<&str> = line.split(',').collect();
            if fields.len() < 12 {
                continue;
            }
            let f = |i: usize| -> Option<f64> { fields.get(i).and_then(|s| s.trim().parse().ok()) };
            let report_date = fields[0].trim().to_string();
            if report_date.is_empty() || report_date.contains("报告期") {
                continue;
            }
            reports.push(FinancialReport {
                stock_code: stock_code.to_string(),
                report_date,
                revenue: f(1),
                net_profit: f(2),
                eps: f(3),
                bps: f(4),
                roe: f(5),
                debt_ratio: f(6),
                gross_margin: f(7),
                net_margin: f(8),
                revenue_yoy: f(9),
                profit_yoy: f(10),
                total_assets: f(11),
                operating_cash_flow: None,
                capital_expenditure: None,
                free_cash_flow: None,
                current_ratio: None,
                quick_ratio: None,
                goodwill: None,
                accounts_receivable: None,
                estimated: Some(false),
            });
        }
        Ok(reports)
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
