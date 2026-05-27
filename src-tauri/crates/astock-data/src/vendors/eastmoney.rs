use crate::error::DataError;
use crate::types::*;
use crate::vendors::StockVendor;
use async_trait::async_trait;
use serde_json::Value;

pub struct EastMoneyVendor {
    pub http: reqwest::Client,
}

/// 构建东方财富股票代码 (1.SH600519, 0.SZ000001)
fn to_em_code(stock_code: &str) -> String {
    let market = if stock_code.starts_with('6') {
        "1"
    } else {
        "0"
    };
    let prefix = if market == "1" { "SH" } else { "SZ" };
    format!("{market}.{prefix}{stock_code}")
}

/// 构建东方财富 secid (1.600519, 0.000001)
fn to_em_secid(stock_code: &str) -> String {
    let market = if stock_code.starts_with('6') {
        "1"
    } else {
        "0"
    };
    format!("{market}.{stock_code}")
}

#[async_trait]
impl StockVendor for EastMoneyVendor {
    async fn get_quote(&self, stock_code: &str) -> Result<StockQuote, DataError> {
        let secid = to_em_secid(stock_code);
        let url = format!(
            "https://push2.eastmoney.com/api/qt/stock/get?secid={secid}&fields=f43,f44,f45,f46,f47,f48,f50,f51,f52,f55,f57,f58,f60,f116,f117,f162,f167,f168,f169,f170,f171"
        );
        let resp = self.http.get(&url).send().await?;
        let json: Value = resp.json().await?;
        let d = &json["data"];
        if d.is_null() {
            return Err(DataError::VendorError {
                vendor: "eastmoney".into(),
                message: "no quote data".into(),
            });
        }
        let f = |key: &str| d[key].as_f64().unwrap_or(0.0);
        Ok(StockQuote {
            code: stock_code.to_string(),
            name: d["f58"].as_str().unwrap_or("").to_string(),
            price: f("f43") / 100.0,
            pre_close: f("f60") / 100.0,
            open: f("f46") / 100.0,
            high: f("f44") / 100.0,
            low: f("f45") / 100.0,
            volume: f("f47"),
            amount: f("f48"),
            change_pct: f("f170") / 100.0,
            turnover_rate: f("f168") / 100.0,
            pe: Some(f("f162") / 100.0).filter(|v| *v > 0.0),
            pb: Some(f("f167") / 100.0).filter(|v| *v > 0.0),
            total_mv: Some(f("f116")).filter(|v| *v > 0.0),
            limit_up: None,
            limit_down: None,
            is_st: d["f57"].as_i64().unwrap_or(0) == 1,
            timestamp: d["f171"]
                .as_i64()
                .map(|t| t.to_string())
                .unwrap_or_default(),
        })
    }

    async fn get_klines(
        &self,
        stock_code: &str,
        period: &str,
        limit: u32,
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
        let url = format!(
            "https://push2his.eastmoney.com/api/qt/stock/kline/get?secid={secid}&fields1=f1,f2,f3,f4,f5,f6&fields2=f51,f52,f53,f54,f55,f56,f57,f58,f59,f60,f61&klt={period_code}&fqt=1&end=20500101&lmt={limit}"
        );

        let resp = self.http.get(&url).send().await?;
        let json: Value = resp.json().await?;

        let klines_raw = json["data"]["klines"]
            .as_array()
            .ok_or_else(|| DataError::ParseError("missing klines array".into()))?;

        klines_raw
            .iter()
            .map(|v| {
                let s = v
                    .as_str()
                    .ok_or_else(|| DataError::ParseError("kline not string".into()))?;
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
                    volume: parse(parts[5]),
                    amount: parse(parts[6]),
                    turnover_rate: Some(parse(parts[10])),
                })
            })
            .collect()
    }

    async fn get_financials(&self, stock_code: &str) -> Result<Vec<FinancialReport>, DataError> {
        let url = format!(
            "https://emweb.securities.eastmoney.com/PC_HSF10/FinanceSummary/FinanceSummary?code={}&type=web",
            to_em_code(stock_code)
        );

        let resp = self
            .http
            .get(&url)
            .header("Referer", "https://emweb.securities.eastmoney.com/")
            .send()
            .await?;
        let json: Value = resp.json().await?;

        let reports = json["data"]["list"]
            .as_array()
            .ok_or_else(|| DataError::ParseError("missing financials list".into()))?;

        reports
            .iter()
            .map(|r| {
                let s = |key: &str| -> &str { r[key].as_str().unwrap_or("") };
                let n = |key: &str| -> Option<f64> { r[key].as_str().and_then(|v| v.parse().ok()) };
                Ok(FinancialReport {
                    stock_code: stock_code.to_string(),
                    report_date: s("REPORT_DATE").to_string(),
                    revenue: n("TOTAL_OPERATE_INCOME"),
                    net_profit: n("PARENT_NETPROFIT"),
                    eps: n("BASIC_EPS"),
                    bps: n("BPS"),
                    roe: n("WEIGHTAVG_ROE"),
                    debt_ratio: n("DEBT_ASSET_RATIO"),
                    gross_margin: n("GROSS_PROFIT_RATIO"),
                    net_margin: n("NETPROFIT_MARGIN"),
                    revenue_yoy: n("TOTAL_OPERATE_INCOME_YOY"),
                    profit_yoy: n("PARENT_NETPROFIT_YOY"),
                })
            })
            .collect()
    }

    async fn get_news(&self, _: &str, _: u32) -> Result<Vec<NewsItem>, DataError> {
        Ok(vec![])
    }

    async fn get_money_flow(&self, stock_code: &str) -> Result<Option<MoneyFlow>, DataError> {
        let secid = to_em_secid(stock_code);
        let url = format!(
            "https://push2his.eastmoney.com/api/qt/stock/fflow/daykline/get?secid={secid}&fields1=f1,f2,f3,f4&fields2=f51,f52,f53,f54,f55,f56&lmt=1"
        );

        let resp = self.http.get(&url).send().await?;
        let json: Value = resp.json().await?;

        let klines = json["data"]["klines"].as_array();
        match klines {
            Some(arr) if !arr.is_empty() => {
                let s = arr[0].as_str().unwrap_or("");
                let parts: Vec<&str> = s.split(',').collect();
                // parts[2] = f53 (主力净流入占比%), 当前 MoneyFlow 结构体不需要此字段
                if parts.len() < 7 {
                    return Ok(None);
                }
                let parse = |s: &str| -> f64 { s.parse().unwrap_or(0.0) };
                Ok(Some(MoneyFlow {
                    date: parts[0].to_string(),
                    main_net_inflow: parse(parts[1]) * 10000.0,
                    super_large_net: parse(parts[3]) * 10000.0,
                    large_net: parse(parts[4]) * 10000.0,
                    medium_net: parse(parts[5]) * 10000.0,
                    small_net: parse(parts[6]) * 10000.0,
                }))
            },
            _ => Ok(None),
        }
    }

    async fn get_dragon_tiger(&self, stock_code: &str) -> Result<Vec<DragonTigerEntry>, DataError> {
        let secid = to_em_secid(stock_code);
        let url = format!(
            "https://push2his.eastmoney.com/api/qt/stock/mmpa/get?secid={secid}&fields1=f1,f2,f3,f4&fields2=f51,f52,f53,f54,f55,f56,f57,f58"
        );

        let resp = self.http.get(&url).send().await?;
        let json: Value = resp.json().await?;

        let entries = match json["data"]["mmpa"].as_array() {
            Some(arr) => arr,
            None => return Ok(vec![]),
        };

        entries
            .iter()
            .map(|e| {
                let s = e.as_str().unwrap_or("");
                let parts: Vec<&str> = s.split(',').collect();
                let parse = |s: &str| -> f64 { s.parse().unwrap_or(0.0) };
                Ok(DragonTigerEntry {
                    stock_code: stock_code.to_string(),
                    date: parts.first().map(|s| s.to_string()).unwrap_or_default(),
                    dept_name: parts.get(1).map(|s| s.to_string()).unwrap_or_default(),
                    buy_amount: parse(parts.get(3).unwrap_or(&"0")),
                    sell_amount: parse(parts.get(4).unwrap_or(&"0")),
                    net_amount: parse(parts.get(5).unwrap_or(&"0")),
                    reason: parts.get(7).map(|s| s.to_string()),
                })
            })
            .collect()
    }

    async fn get_lockup_schedule(
        &self,
        stock_code: &str,
    ) -> Result<Vec<LockupSchedule>, DataError> {
        let url = format!(
            "https://datacenter-web.eastmoney.com/api/data/v1/get?reportName=RPTA_WEB_LOCKUP&columns=SECURITY_CODE,SECURITY_NAME_ABBR,UNLOCK_DATE,UNLOCK_SHARES,PLACING_RATIO,HOLDER_NAME&filter=(SECURITY_CODE=\"{stock_code}\")&pageSize=20&sortColumns=UNLOCK_DATE&pageNumber=1"
        );

        let resp = self.http.get(url).send().await?;
        let json: Value = resp.json().await?;

        let rows = match json["result"]["data"].as_array() {
            Some(arr) => arr,
            None => return Ok(vec![]),
        };

        Ok(rows
            .iter()
            .map(|r| LockupSchedule {
                stock_code: stock_code.to_string(),
                stock_name: r["SECURITY_NAME_ABBR"].as_str().unwrap_or("").to_string(),
                unlock_date: r["UNLOCK_DATE"].as_str().unwrap_or("").to_string(),
                unlock_shares: r["UNLOCK_SHARES"].as_f64().unwrap_or(0.0),
                unlock_ratio: r["PLACING_RATIO"].as_f64().unwrap_or(0.0),
                shareholder: r["HOLDER_NAME"].as_str().map(|s| s.to_string()),
            })
            .collect())
    }

    /// 获取融资融券数据
    async fn get_margin_data(&self, stock_code: &str) -> Result<Option<MarginData>, DataError> {
        let secid = to_em_secid(stock_code);
        let url = format!(
            "https://push2his.eastmoney.com/api/qt/stock/margin/get?secid={secid}&fields1=f1,f2,f3,f4,f5&fields2=f51,f52,f53,f54,f55"
        );

        let resp = self.http.get(&url).send().await?;
        let json: Value = resp.json().await?;

        let data = &json["data"];
        let parse_num =
            |v: &Value| -> f64 { v.as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0) };

        if data.is_null() || data["f51"].is_null() {
            return Ok(None);
        }

        Ok(Some(MarginData {
            stock_code: stock_code.to_string(),
            date: data["f51"].as_str().unwrap_or("").to_string(),
            margin_buy: parse_num(&data["f52"]) * 10000.0,
            margin_balance: parse_num(&data["f53"]) * 10000.0,
            short_sell_volume: parse_num(&data["f54"]) * 100.0,
            short_balance: parse_num(&data["f55"]) * 100.0,
        }))
    }

    async fn get_north_bound_holding(
        &self,
        stock_code: &str,
    ) -> Result<Option<NorthBoundHolding>, DataError> {
        let secid = to_em_secid(stock_code);
        // 东方财富北向资金个股级别API: 通过个股资金流向K线接口获取
        // klt=3 代表日级别北向资金数据
        let url = format!(
            "https://push2his.eastmoney.com/api/qt/stock/fflow/daykline/get?secid={secid}&fields1=f1,f2,f3&fields2=f51,f52,f53&lmt=1&klt=3"
        );
        let resp = self.http.get(&url).send().await?;
        let json: Value = resp.json().await?;

        if let Some(arr) = json["data"]["klines"].as_array() {
            if let Some(line) = arr.first().and_then(|v| v.as_str()) {
                let parts: Vec<&str> = line.split(',').collect();
                if parts.len() >= 3 {
                    let holding_shares: f64 = parts[1].parse().unwrap_or(0.0);
                    let holding_ratio: f64 = parts[2].parse().unwrap_or(0.0);
                    // 变动数量通过与前一日差值计算（此处返回0，由调用方自行计算）
                    return Ok(Some(NorthBoundHolding {
                        stock_code: stock_code.to_string(),
                        date: parts[0].to_string(),
                        holding_shares,
                        holding_ratio,
                        change_shares: 0.0,
                    }));
                }
            }
        }
        // 北向资金个股数据可能不可用（部分股票无数据），返回 None 而非错误
        Ok(None)
    }

    async fn get_sector_info(&self, stock_code: &str) -> Result<Option<SectorInfo>, DataError> {
        // 通过东方财富行情API获取个股的行业和概念板块信息
        // f158=申万一级行业, f159=申万二级行业, f160=概念板块, f161/f162=其他分类
        let url = format!(
            "https://push2.eastmoney.com/api/qt/stock/get?secid={}&fields=f158,f159,f160",
            to_em_secid(stock_code)
        );
        let resp = self.http.get(&url).send().await?;
        let json: Value = resp.json().await?;

        let data = &json["data"];
        let sector_name = data["f158"].as_str().unwrap_or("").to_string();
        let sub_sector = data["f159"].as_str().unwrap_or("").to_string();
        let concept_tags: Vec<String> = data["f160"]
            .as_str()
            .map(|s| s.split(',').map(|t| t.trim().to_string()).collect())
            .unwrap_or_default();

        if sector_name.is_empty() && concept_tags.is_empty() {
            return Ok(None);
        }

        Ok(Some(SectorInfo {
            stock_code: stock_code.to_string(),
            sector_name,
            sub_sector,
            concept_tags,
        }))
    }

    async fn get_shareholder_trades(
        &self,
        stock_code: &str,
    ) -> Result<Vec<ShareholderTrade>, DataError> {
        // 东方财富数据中心: 大股东增减持数据
        // SECURITY_CODE=股票代码, CHANGE_DATE=变动日期, SHAREHD_NAME=股东名称
        // CHANGE_TYPE=变动类型(增持/减持), CHANGE_NUM=变动数量, CHANGE_PRICE=变动均价
        let url = format!(
            "https://datacenter-web.eastmoney.com/api/data/v1/get?reportName=RPTA_WEB_MAJORHOLDERS_TRADE&columns=SECURITY_CODE,CHANGE_DATE,SHAREHD_NAME,CHANGE_TYPE,CHANGE_NUM,CHANGE_PRICE,CHANGE_REASON&filter=(SECURITY_CODE=\"{stock_code}\")&pageSize=20&pageNumber=1"
        );
        let resp = self.http.get(&url).send().await?;
        let json: Value = resp.json().await?;

        let rows = match json["result"]["data"].as_array() {
            Some(arr) => arr,
            None => return Ok(vec![]),
        };

        rows.iter()
            .map(|r| {
                Ok(ShareholderTrade {
                    stock_code: stock_code.to_string(),
                    date: r["CHANGE_DATE"].as_str().unwrap_or("").to_string(),
                    shareholder_name: r["SHAREHD_NAME"].as_str().unwrap_or("").to_string(),
                    trade_type: r["CHANGE_TYPE"].as_str().unwrap_or("").to_string(),
                    shares: r["CHANGE_NUM"].as_f64().unwrap_or(0.0),
                    price: r["CHANGE_PRICE"].as_f64().unwrap_or(0.0),
                    reason: r["CHANGE_REASON"].as_str().map(|s| s.to_string()),
                })
            })
            .collect()
    }

    async fn get_dividend_records(
        &self,
        stock_code: &str,
    ) -> Result<Vec<DividendRecord>, DataError> {
        // 东方财富数据中心: 分红送配数据
        // SECURITY_CODE=股票代码, EX_DIVIDEND_DATE=除权除息日
        // DIVIDEND_PER_SHARE=每股分红, BONUS_SHARE_RATIO=送转比例, RECORD_DATE=股权登记日
        let url = format!(
            "https://datacenter-web.eastmoney.com/api/data/v1/get?reportName=RPTA_WEB_DIVIDEND&columns=SECURITY_CODE,EX_DIVIDEND_DATE,DIVIDEND_PER_SHARE,BONUS_SHARE_RATIO,RECORD_DATE&filter=(SECURITY_CODE=\"{stock_code}\")&pageSize=10&pageNumber=1"
        );
        let resp = self.http.get(&url).send().await?;
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

    async fn search_stock(&self, keyword: &str) -> Result<Vec<StockSearchResult>, DataError> {
        let url = format!(
            "https://searchadapter.eastmoney.com/api/suggest/get?input={}&type=14&count=20",
            urlencoding::encode(keyword)
        );

        let resp = self.http.get(&url).send().await?;
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

        let resp = self
            .http
            .get(&url)
            .header("Referer", "https://data.eastmoney.com/")
            .send()
            .await?;

        let json: Value = resp.json().await?;

        let reports = match json["data"].as_array() {
            Some(arr) => arr,
            None => return Ok(vec![]),
        };

        Ok(reports
            .iter()
            .map(|r| {
                let mut eps_forecast = Vec::new();
                if let Some(eps) = r["predictThisYearEps"].as_str() {
                    if let Ok(val) = eps.parse::<f64>() {
                        eps_forecast.push(EpsForecast {
                            year: "今年".into(),
                            eps: Some(val),
                        });
                    }
                }
                if let Some(eps) = r["predictNextYearEps"].as_str() {
                    if let Ok(val) = eps.parse::<f64>() {
                        eps_forecast.push(EpsForecast {
                            year: "明年".into(),
                            eps: Some(val),
                        });
                    }
                }
                if let Some(eps) = r["predictNextTwoYearEps"].as_str() {
                    if let Ok(val) = eps.parse::<f64>() {
                        eps_forecast.push(EpsForecast {
                            year: "后年".into(),
                            eps: Some(val),
                        });
                    }
                }

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
                    target_price: None,
                    eps_forecast,
                    publish_date: r["publishDate"].as_str().unwrap_or("").to_string(),
                    pdf_url,
                }
            })
            .collect())
    }

    async fn get_market_dragon_tiger(&self) -> Result<Vec<MarketDragonTiger>, DataError> {
        let url = "https://datacenter-web.eastmoney.com/api/data/v1/get?reportName=RPT_DAILYBOARD_DETAILS_NEW&columns=SECURITY_CODE,SECURITY_NAME_ABBR,TRADE_DATE,BUY_AMOUNT,SELL_AMOUNT,NET_BUY,CHANGE_REASON&sortColumns=NET_BUY&sortTypes=-1&pageSize=30&pageNumber=1";

        let resp = self.http.get(url).send().await?;
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
        let url = "https://np-listapi.eastmoney.com/comm/web/getNewsByColumns?client=web&biz=web_news_col&column=250&order=1&needInteractData=0&page_index=1&page_size=20";

        let resp = self
            .http
            .get(url)
            .header("Referer", "https://finance.eastmoney.com/")
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .send()
            .await?;

        let json: Value = resp.json().await?;

        let items = match json["data"]["list"].as_array() {
            Some(arr) => arr,
            None => match json["data"].as_array() {
                Some(arr) => arr,
                None => return Ok(vec![]),
            },
        };

        Ok(items
            .iter()
            .filter_map(|item| {
                let title = item.get("title")?.as_str()?.to_string();
                let content = item
                    .get("digest")
                    .or_else(|| item.get("content"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let publish_time = item
                    .get("showTime")
                    .or_else(|| item.get("publish_time"))
                    .or_else(|| item.get("ctime"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let source = item
                    .get("source")
                    .or_else(|| item.get("mediaName"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                Some(ClsFlashItem {
                    title,
                    content,
                    publish_time,
                    source,
                })
            })
            .collect())
    }
}
