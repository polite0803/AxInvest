use chrono::{Datelike, NaiveDate, Timelike, Weekday};

/// 判断是否为A股交易日
pub fn is_trading_day(date: &NaiveDate) -> bool {
    let w = date.weekday();
    // 周末不交易
    if w == Weekday::Sat || w == Weekday::Sun {
        return false;
    }

    // 硬编码2025-2026年A股节假日（简化版，不含临时休市）
    let holidays = [
        // 2025年
        "2025-01-01", // 元旦
        "2025-01-28",
        "2025-01-29",
        "2025-01-30",
        "2025-01-31",
        "2025-02-03",
        "2025-02-04", // 春节
        "2025-04-04",
        "2025-04-07", // 清明
        "2025-05-01",
        "2025-05-02",
        "2025-05-05", // 劳动节
        "2025-06-02", // 端午
        "2025-09-15",
        "2025-09-16", // 中秋
        "2025-10-01",
        "2025-10-02",
        "2025-10-03",
        "2025-10-06",
        "2025-10-07",
        "2025-10-08", // 国庆
        // 2026年
        "2026-01-01",
        "2026-01-02",
        "2026-02-16",
        "2026-02-17",
        "2026-02-18",
        "2026-02-19",
        "2026-02-20",
        "2026-02-23",
        "2026-02-24",
        "2026-03-23",
        "2026-03-24",
        "2026-04-20",
        "2026-04-21",
        "2026-05-18",
        "2026-05-19",
        "2026-06-08",
        "2026-06-09",
        "2026-09-21",
        "2026-09-22",
        "2026-09-28",
        "2026-09-29",
        "2026-09-30",
        "2026-10-01",
        "2026-10-02",
    ];

    let date_str = date.format("%Y-%m-%d").to_string();
    if holidays.contains(&date_str.as_str()) {
        return false;
    }

    // 调休工作日（周末但上班）
    let workdays = [
        "2025-02-08", // 春节调休
        "2025-04-27", // 清明调休
        "2025-09-28", // 中秋调休
        "2025-10-11", // 国庆调休
    ];

    if workdays.contains(&date_str.as_str()) {
        return true;
    }

    true
}

/// 判断当前是否为交易时间
pub fn is_trading_time() -> bool {
    let now = chrono::Utc::now();
    let today = now.date_naive();

    if !is_trading_day(&today) {
        return false;
    }

    // 获取北京时间（UTC+8）
    let hour = (now.hour() + 8) % 24;
    let minute = now.minute();

    // 上午 9:30-11:30
    if hour == 9 && minute >= 30 {
        return true;
    }
    if hour == 10 {
        return true;
    }
    if hour == 11 && minute <= 30 {
        return true;
    }

    // 下午 13:00-15:00
    if hour == 13 {
        return true;
    }
    if hour == 14 {
        return true;
    }
    if hour == 15 && minute == 0 {
        return true;
    }

    false
}

/// 判断当前是否为午休时间
pub fn is_lunch_break() -> bool {
    let now = chrono::Utc::now();
    let hour = (now.hour() + 8) % 24;
    hour == 11 || hour == 12
}

/// 从东方财富 API 获取最新交易日历
pub async fn fetch_holiday_calendar() -> Result<Vec<String>, String> {
    let url = "https://datacenter-web.eastmoney.com/api/data/v1/get?reportName=RPTA_WEB_TRADE_CALENDAR&columns=TRADE_DATE,IS_TRADING_DAY&pageSize=365&pageNumber=1";
    let resp = reqwest::get(url).await.map_err(|e| format!("获取交易日历失败: {}", e))?;
    let json: serde_json::Value = resp.json().await.map_err(|e| format!("解析失败: {}", e))?;

    let holidays: Vec<String> = json["result"]["data"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|d| {
            let is_trading = d["IS_TRADING_DAY"].as_str().unwrap_or("1") == "0";
            let date = d["TRADE_DATE"].as_str().unwrap_or("").to_string();
            if !date.is_empty() && is_trading { Some(date) } else { None }
        })
        .collect();

    Ok(holidays)
}

/// 获取距离下一个交易时间的描述
pub fn next_trading_time_desc() -> String {
    if is_trading_time() {
        return "交易中".to_string();
    }
    if is_lunch_break() {
        return "午休中，13:00恢复".to_string();
    }

    let now = chrono::Utc::now();
    let hour = (now.hour() + 8) % 24;
    if hour < 9 || (hour == 9 && now.minute() < 30) {
        return "距开盘".to_string();
    }
    if hour >= 15 {
        return "已收盘".to_string();
    }
    "非交易时段".to_string()
}
