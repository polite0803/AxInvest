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

#[async_trait]
impl StockVendor for TencentVendor {
    async fn get_quote(&self, stock_code: &str) -> Result<StockQuote, DataError> {
        let tc_code = to_tencent_code(stock_code);
        let url = format!("http://qt.gtimg.cn/q={tc_code}");
        let resp = self.http.get(&url).send().await?;
        let text = resp.text().await?;
        parse_quote(&text)
    }

    // 以下方法由其他 vendor 承担，此处返回空
    async fn get_klines(&self, _: &str, _: &str, _: u32) -> Result<Vec<KLine>, DataError> {
        Ok(vec![])
    }

    async fn get_financials(&self, _: &str) -> Result<Vec<FinancialReport>, DataError> {
        Ok(vec![])
    }

    async fn get_news(&self, _: &str, _: u32) -> Result<Vec<NewsItem>, DataError> {
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
