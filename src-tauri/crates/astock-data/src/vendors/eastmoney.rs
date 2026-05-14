use async_trait::async_trait;
use crate::error::DataError;
use crate::types::*;
use crate::vendors::StockVendor;
use reqwest::Client;
use serde_json::Value;

pub struct EastMoneyVendor;

/// 构建东方财富股票代码 (1.SH600519, 0.SZ000001)
fn to_em_code(stock_code: &str) -> String {
    let market = match stock_code.chars().next() {
        Some('6') => "1",
        Some('0') | Some('2') => "0",
        Some('3') => "0",
        Some('8') | Some('4') => "0",
        _ => "0",
    };
    format!(
        "{}.{}{}",
        market,
        if market == "1" { "SH" } else { "SZ" },
        stock_code
    )
}

/// 构建东方财富 secid (1.600519, 0.000001)
fn to_em_secid(stock_code: &str) -> String {
    let market = match stock_code.chars().next() {
        Some('6') => "1",
        Some('0') | Some('2') => "0",
        Some('3') => "0",
        Some('8') | Some('4') => "0",
        _ => "0",
    };
    format!("{}.{}", market, stock_code)
}

#[async_trait]
impl StockVendor for EastMoneyVendor {
    fn name(&self) -> &'static str { "eastmoney" }

    async fn get_quote(&self, _: &str) -> Result<StockQuote, DataError> {
        Err(DataError::VendorError {
            vendor: "eastmoney".into(),
            message: "quote handled by tencent vendor".into(),
        })
    }

    async fn get_klines(
        &self,
        stock_code: &str,
        period: &str,
        limit: u32,
    ) -> Result<Vec<KLine>, DataError> {
        let client = Client::new();
        let period_code = match period {
            "daily" | "101" => "101",
            "weekly" | "102" => "102",
            "monthly" | "103" => "103",
            _ => "101",
        };
        let secid = to_em_secid(stock_code);
        let url = format!(
            "https://push2his.eastmoney.com/api/qt/stock/kline/get?secid={}&fields1=f1,f2,f3,f4,f5,f6&fields2=f51,f52,f53,f54,f55,f56,f57,f58,f59,f60,f61&klt={}&fqt=1&end=20500101&lmt={}",
            secid, period_code, limit
        );

        let resp = client.get(&url).send().await?;
        let json: Value = resp.json().await?;

        let klines_raw = json["data"]["klines"]
            .as_array()
            .ok_or_else(|| DataError::ParseError("missing klines array".into()))?;

        klines_raw
            .iter()
            .map(|v| {
                let s = v.as_str().ok_or_else(|| DataError::ParseError("kline not string".into()))?;
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

    async fn get_financials(
        &self,
        stock_code: &str,
    ) -> Result<Vec<FinancialReport>, DataError> {
        let client = Client::new();
        let url = format!(
            "https://emweb.securities.eastmoney.com/PC_HSF10/FinanceSummary/FinanceSummary?code={}&type=web",
            to_em_code(stock_code)
        );

        let resp = client
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
        let client = Client::new();
        let secid = to_em_secid(stock_code);
        let url = format!(
            "https://push2his.eastmoney.com/api/qt/stock/fflow/daykline/get?secid={}&fields1=f1,f2,f3,f4&fields2=f51,f52,f53,f54,f55,f56&lmt=1",
            secid
        );

        let resp = client.get(&url).send().await?;
        let json: Value = resp.json().await?;

        let klines = json["data"]["klines"].as_array();
        match klines {
            Some(arr) if !arr.is_empty() => {
                let s = arr[0].as_str().unwrap_or("");
                let parts: Vec<&str> = s.split(',').collect();
                if parts.len() < 6 {
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
            }
            _ => Ok(None),
        }
    }

    async fn get_dragon_tiger(
        &self,
        stock_code: &str,
    ) -> Result<Vec<DragonTigerEntry>, DataError> {
        let client = Client::new();
        let secid = to_em_secid(stock_code);
        let url = format!(
            "https://push2his.eastmoney.com/api/qt/stock/mmpa/get?secid={}&fields1=f1,f2,f3,f4&fields2=f51,f52,f53,f54,f55,f56,f57,f58",
            secid
        );

        let resp = client.get(&url).send().await?;
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
                    date: parts.get(0).map(|s| s.to_string()).unwrap_or_default(),
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
        let client = Client::new();
        let url = format!(
            "https://datacenter-web.eastmoney.com/api/data/v1/get?reportName=RPTA_WEB_LOCKUP&columns=SECURITY_CODE,SECURITY_NAME_ABBR,UNLOCK_DATE,UNLOCK_SHARES,PLACING_RATIO,HOLDER_NAME&filter=(SECURITY_CODE=\"{}\")&pageSize=20&sortColumns=UNLOCK_DATE&pageNumber=1",
            stock_code
        );

        let resp = client.get(&url).send().await?;
        let json: Value = resp.json().await?;

        let rows = match json["result"]["data"].as_array() {
            Some(arr) => arr,
            None => return Ok(vec![]),
        };

        rows.iter()
            .map(|r| {
                Ok(LockupSchedule {
                    stock_code: stock_code.to_string(),
                    stock_name: r["SECURITY_NAME_ABBR"].as_str().unwrap_or("").to_string(),
                    unlock_date: r["UNLOCK_DATE"].as_str().unwrap_or("").to_string(),
                    unlock_shares: r["UNLOCK_SHARES"].as_f64().unwrap_or(0.0),
                    unlock_ratio: r["PLACING_RATIO"].as_f64().unwrap_or(0.0),
                    shareholder: r["HOLDER_NAME"].as_str().map(|s| s.to_string()),
                })
            })
            .collect()
    }

    async fn search_stock(
        &self,
        keyword: &str,
    ) -> Result<Vec<StockSearchResult>, DataError> {
        let client = Client::new();
        let url = format!(
            "https://searchadapter.eastmoney.com/api/suggest/get?input={}&type=14&count=20",
            urlencoding::encode(keyword)
        );

        let resp = client.get(&url).send().await?;
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
}
