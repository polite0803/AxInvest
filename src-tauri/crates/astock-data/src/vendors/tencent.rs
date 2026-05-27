use crate::error::DataError;
use crate::types::*;
use crate::vendors::StockVendor;
use async_trait::async_trait;

pub struct TencentVendor {
    pub http: reqwest::Client,
}

/// 将 AxInvest 股票代码转为腾讯财经格式
/// 600519 → sh600519, 000001 → sz000001, 300750 → sz300750
fn to_tencent_code(stock_code: &str) -> String {
    let prefix = match stock_code.chars().next() {
        Some('6') => "sh",
        Some('0') | Some('3') | Some('2') => "sz",
        Some('8') | Some('4') => "bj",
        _ => "sz",
    };
    format!("{prefix}{stock_code}")
}

/// 解析腾讯财经实时行情响应
fn parse_quote(raw: &str) -> Result<StockQuote, DataError> {
    // 格式: v_sh600519="1~贵州茅台~600519~1680.00~1650.00~..."
    let start = raw
        .find('"')
        .ok_or_else(|| DataError::ParseError("no opening quote".into()))?;
    let end = raw[start + 1..]
        .find('"')
        .ok_or_else(|| DataError::ParseError("no closing quote".into()))?;
    let data = &raw[start + 1..start + 1 + end];
    let fields: Vec<&str> = data.split('~').collect();

    if fields.len() < 49 {
        return Err(DataError::ParseError(format!("expected >=49 fields, got {}", fields.len())));
    }

    let parse = |s: &str| -> f64 { s.parse().unwrap_or(0.0) };
    let parse_opt = |s: &str| -> Option<f64> {
        let v: f64 = s.parse().ok()?;
        if v == 0.0 {
            None
        } else {
            Some(v)
        }
    };

    // 从股票名称检测 ST 状态
    let name = fields[1].to_string();
    let is_st = name.contains("ST") || name.contains("*ST");

    Ok(StockQuote {
        code: fields[2].to_string(),
        name,
        price: parse(fields[3]),
        pre_close: parse(fields[4]),
        open: parse(fields[5]),
        high: parse(fields[33]),
        low: parse(fields[34]),
        volume: parse(fields[6]) * 100.0,
        amount: parse(fields[37]) * 10000.0,
        change_pct: parse(fields[32]),
        turnover_rate: parse(fields[38]),
        pe: parse_opt(fields[39]),
        pb: parse_opt(fields[46]),
        total_mv: parse_opt(fields[45]).map(|v| v * 10000.0),
        limit_up: {
            let v = parse(fields[47]);
            if v > 0.0 {
                Some(v)
            } else {
                None
            }
        },
        limit_down: {
            let v = parse(fields[48]);
            if v > 0.0 {
                Some(v)
            } else {
                None
            }
        },
        is_st,
        timestamp: chrono::Utc::now().to_rfc3339(),
    })
}

/// 解析腾讯财经 K 线 JSON 响应
/// API: http://web.ifzq.gtimg.cn/appstock/app/fqkline/get?param=sz000001,day,,,120,qfq
fn parse_klines(raw: &str, _stock_code: &str) -> Result<Vec<KLine>, DataError> {
    let json: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| DataError::ParseError(format!("K线JSON解析失败: {e}")))?;
    // 路径: data.{code}.{qfqday|qfqweek|qfqmonth} 或 data.{code}.day/week/month
    let data = &json["data"];
    let code_key = data
        .as_object()
        .and_then(|obj| {
            obj.keys()
                .find(|k| k.starts_with("sz") || k.starts_with("sh") || k.starts_with("bj"))
        })
        .cloned()
        .unwrap_or_default();
    if code_key.is_empty() {
        return Err(DataError::ParseError("K线数据中未找到股票代码键".into()));
    }
    let stock_data = &data[&code_key];
    // 尝试各种可能的键名
    let kline_list = stock_data["qfqday"]
        .as_array()
        .or_else(|| stock_data["day"].as_array())
        .or_else(|| stock_data["qfqweek"].as_array())
        .or_else(|| stock_data["week"].as_array())
        .or_else(|| stock_data["qfqmonth"].as_array())
        .or_else(|| stock_data["month"].as_array())
        .ok_or_else(|| DataError::ParseError("未找到K线数组".into()))?;

    let mut result = Vec::new();
    for item in kline_list {
        let arr = item
            .as_array()
            .ok_or_else(|| DataError::ParseError("K线项不是数组".into()))?;
        if arr.len() < 6 {
            continue;
        }
        let parse = |i: usize| -> f64 {
            arr.get(i)
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0)
        };
        result.push(KLine {
            date: arr[0].as_str().unwrap_or("").to_string(),
            open: parse(1),
            close: parse(2),
            high: parse(3),
            low: parse(4),
            volume: parse(5),
            amount: parse(6),
            turnover_rate: None,
        });
    }
    Ok(result)
}

#[async_trait]
impl StockVendor for TencentVendor {
    async fn get_quote(&self, stock_code: &str) -> Result<StockQuote, DataError> {
        let tc_code = to_tencent_code(stock_code);
        let url = format!("https://qt.gtimg.cn/q={tc_code}");
        let resp = self.http.get(&url).send().await?;
        let text = resp.text().await?;
        parse_quote(&text)
    }

    // 以下方法由腾讯财经 API 提供
    async fn get_klines(
        &self,
        stock_code: &str,
        period: &str,
        limit: u32,
    ) -> Result<Vec<KLine>, DataError> {
        let tc_code = to_tencent_code(stock_code);
        let period_code = match period {
            "weekly" => "week",
            "monthly" => "month",
            _ => "day",
        };
        let url = format!(
            "https://web.ifzq.gtimg.cn/appstock/app/fqkline/get?param={tc_code},{period_code},,,{limit},qfq"
        );
        let resp = self.http.get(&url).send().await?;
        let body = resp.text().await?;
        parse_klines(&body, stock_code)
    }

    async fn get_financials(&self, _: &str) -> Result<Vec<FinancialReport>, DataError> {
        // 腾讯无直接财务 API，由其他 vendor 承担
        Ok(vec![])
    }

    async fn get_news(&self, _: &str, _: u32) -> Result<Vec<NewsItem>, DataError> {
        // 腾讯无直接新闻 API，由其他 vendor 承担
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
}
