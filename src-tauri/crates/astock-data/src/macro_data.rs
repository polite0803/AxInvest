//! 宏观经济数据 — 数据结构和获取接口
//!
//! 覆盖：GDP、CPI、PMI、利率（LPR/MLF/Shibor）、社融、货币供应量（M0/M1/M2）、
//! 进出口、工业增加值、固定投资、消费零售、外汇储备、汇率等。
//!
//! ## Vendor 适配
//!
//! 数据实际从以下渠道获取：
//! - **国家统计局 / 央行**（通过 neodata / eastmoney / akshare 代理）
//! - 目前为数据结构层 + 占位实现，TODO: 接入真实数据源

use serde::{Deserialize, Serialize};

use chrono::Datelike;

use crate::error::DataError;

/// 宏观经济数据点
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacroDataPoint {
    /// 指标名（如 "gdp", "cpi", "pmi"）
    pub indicator: String,
    /// 指标显示名（如 "国内生产总值(GDP)"）
    pub display_name: String,
    /// 统计期间（如 "2025Q3", "2025-09", "2025"）
    pub period: String,
    /// 数值
    pub value: f64,
    /// 同比（%），部分指标有
    pub yoy: Option<f64>,
    /// 环比（%），部分指标有
    pub mom: Option<f64>,
    /// 单位
    pub unit: String,
    /// 数据来源
    pub source: String,
    /// 发布时间
    pub release_date: Option<String>,
}

/// 宏观数据集合（一次性获取全部可用数据）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacroDataSnapshot {
    /// 数据日期（获取时的有效日期）
    pub snapshot_date: String,
    /// 最新 GDP 数据
    pub gdp: Option<MacroDataPoint>,
    /// 最新 CPI 数据
    pub cpi: Option<MacroDataPoint>,
    /// 最新 PPI 数据
    pub ppi: Option<MacroDataPoint>,
    /// 最新 PMI 数据（制造业）
    pub pmi_manufacturing: Option<MacroDataPoint>,
    /// 最新 PMI 数据（非制造业）
    pub pmi_non_manufacturing: Option<MacroDataPoint>,
    /// 最新 LPR 1年期
    pub lpr_1y: Option<MacroDataPoint>,
    /// 最新 LPR 5年期
    pub lpr_5y: Option<MacroDataPoint>,
    /// 最新 Shibor 隔夜
    pub shibor_on: Option<MacroDataPoint>,
    /// 最新 MLF 利率
    pub mlf_rate: Option<MacroDataPoint>,
    /// 社会融资规模存量同比
    pub social_financing_yoy: Option<MacroDataPoint>,
    /// M2 货币供应量同比
    pub m2_yoy: Option<MacroDataPoint>,
    /// M1 货币供应量同比
    pub m1_yoy: Option<MacroDataPoint>,
    /// 新增人民币贷款
    pub new_loan: Option<MacroDataPoint>,
    /// 出口同比
    pub export_yoy: Option<MacroDataPoint>,
    /// 进口同比
    pub import_yoy: Option<MacroDataPoint>,
    /// 工业增加值同比
    pub industrial_output: Option<MacroDataPoint>,
    /// 社会消费品零售总额同比
    pub retail_sales: Option<MacroDataPoint>,
    /// 固定资产投资累计同比
    pub fixed_asset_investment: Option<MacroDataPoint>,
    /// 外汇储备（亿美元）
    pub forex_reserve: Option<MacroDataPoint>,
    /// 美元/人民币汇率（中间价）
    pub usd_cny: Option<MacroDataPoint>,
    /// 全部数据聚合列表（方便循环遍历）
    #[serde(skip)]
    pub all: Vec<MacroDataPoint>,
}

impl MacroDataSnapshot {
    /// 创建一个空的快照
    pub fn empty(date: &str) -> Self {
        Self {
            snapshot_date: date.to_string(),
            gdp: None,
            cpi: None,
            ppi: None,
            pmi_manufacturing: None,
            pmi_non_manufacturing: None,
            lpr_1y: None,
            lpr_5y: None,
            shibor_on: None,
            mlf_rate: None,
            social_financing_yoy: None,
            m2_yoy: None,
            m1_yoy: None,
            new_loan: None,
            export_yoy: None,
            import_yoy: None,
            industrial_output: None,
            retail_sales: None,
            fixed_asset_investment: None,
            forex_reserve: None,
            usd_cny: None,
            all: Vec::new(),
        }
    }
}

/// 宏观数据客户端 —— 东财数据中心的**真实历史序列**。
///
/// 2026-10-03 由 mock 改为实源。可用的四条 reportName 是逐个试出来的：
/// `RPT_ECONOMY_CPI` / `_PPI` / `_PMI` / `_GDP` 回数据；而 M2、LPR、社融、固投、
/// 进出口、工业增加值等 **18 个候选名一律回「报表配置不存在」**。另有两次报错是
/// `9501 排序列不存在` —— 排序列实名是 `REPORT_DATE`（带下划线），写成 `REPORTDATE` 必失败。
/// ⇒ 本结构**只承诺这四条**；取不到的指标保持 `None`，不得用邻近指标顶替。
pub struct MacroDataClient {
    http: reqwest::Client,
}

impl Default for MacroDataClient {
    fn default() -> Self {
        Self::new()
    }
}

/// 已实测回包的宏观序列（`indicator` → `reportName`）。
const MACRO_SERIES: &[(&str, &str)] = &[
    ("cpi", "RPT_ECONOMY_CPI"),
    ("ppi", "RPT_ECONOMY_PPI"),
    ("pmi", "RPT_ECONOMY_PMI"),
    ("gdp", "RPT_ECONOMY_GDP"),
];

/// 一条宏观序列的**保守可得日**（按发布节奏推断）。
///
/// `REPORT_DATE` 是**统计期**不是发布日：CPI/PPI 实际在次月 9–12 日发布。
/// 若按统计期裁，as-of 回放会拿到「当时尚未公布」的数字 = 前视泄露，
/// 与 `FinancialReport::effective_disclosure_date` 是同一族缺陷。
/// 现场证据：本机 2026-10-03 取 CPI，最新一期是 **2026-08**（9 月值尚未发布）。
/// ⇒ 这是**规则推断**而非公告日实测，来源写进 `source` 里，不得让下游当成公告日。
pub fn macro_available_at(indicator: &str, report_date: &str) -> Option<chrono::NaiveDate> {
    let d = chrono::NaiveDate::parse_from_str(report_date.get(..10)?, "%Y-%m-%d").ok()?;
    let (y, m) = (d.year(), d.month());
    // 次月固定日（跨年进位）。不写 `m + shift` 这类混合类型的算术：
    // `year()` 返 i32、`month()` 返 u32，闭包带 i64 参数会把类型搅乱。
    let day_of_next_month = |day: u32| -> Option<chrono::NaiveDate> {
        if m == 12 {
            chrono::NaiveDate::from_ymd_opt(y + 1, 1, day)
        } else {
            chrono::NaiveDate::from_ymd_opt(y, m + 1, day)
        }
    };
    match indicator {
        // 次月 15 日：覆盖 9–12 日的发布窗口并留余量
        "cpi" | "ppi" => day_of_next_month(15),
        // 当月最后一个工作日发布 ⇒ 取月末 +2 天为保守界
        "pmi" => {
            let last_day = match m {
                1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
                4 | 6 | 9 | 11 => 30,
                _ => {
                    if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                        29
                    } else {
                        28
                    }
                },
            };
            chrono::NaiveDate::from_ymd_opt(y, m, last_day).map(|e| e + chrono::Duration::days(2))
        },
        // GDP 为季度累计值，季后次月 20 日前后公布初值
        "gdp" => day_of_next_month(20),
        _ => None,
    }
}

/// 数据来源标记：把「发布日是推断的」写进串里，避免下游误当公告日。
fn macro_source_name(indicator: &str) -> String {
    let rp = MACRO_SERIES
        .iter()
        .find(|(k, _)| *k == indicator)
        .map_or("RPT_ECONOMY_UNKNOWN", |(_, rp)| *rp);
    format!("eastmoney:datacenter/{rp}(发布日按规则推断)")
}

fn macro_num(v: &serde_json::Value, key: &str) -> Option<f64> {
    v.get(key).and_then(|x| match x {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    })
}

fn macro_period(v: &serde_json::Value) -> String {
    v.get("TIME").and_then(|x| x.as_str()).unwrap_or("").to_string()
}

/// 把响应行解析成数据点。**响应顺序不可信**（实测不带排序时返回 2013、2021、2009 的乱序），
/// 因此本函数只做映射，「取哪一期」由 [`pick_published`] 决定。
fn parse_macro_rows(indicator: &str, rows: &[serde_json::Value]) -> Vec<MacroDataPoint> {
    let mut out: Vec<MacroDataPoint> = Vec::new();
    for r in rows {
        let Some(rd) = r
            .get("REPORT_DATE")
            .and_then(|x| x.as_str())
            .map(|s| s.get(..10).unwrap_or(s).to_string())
        else {
            continue;
        };
        let available =
            macro_available_at(indicator, &rd).map(|d| d.format("%Y-%m-%d").to_string());
        // CPI 的 `NATIONAL_SEQUENTIAL`（环比）**不取**：实测 2013 行是 1.0103（比率形态）、
        // 2026 行是 0.4（百分点形态）—— 同一列跨年换了口径，混用会造出「环比 +101%」这种假读数。
        let (display, value, yoy) = match indicator {
            "cpi" => ("居民消费价格指数(CPI) 同比", macro_num(r, "NATIONAL_SAME"), None),
            "ppi" => ("工业生产者出厂价格指数(PPI) 同比", macro_num(r, "BASE_SAME"), None),
            "pmi" => {
                ("制造业采购经理指数(PMI)", macro_num(r, "MAKE_INDEX"), macro_num(r, "MAKE_SAME"))
            },
            "gdp" => ("GDP 累计同比", macro_num(r, "SUM_SAME"), None),
            _ => continue,
        };
        let Some(v) = value else { continue };
        // 同比型指标：value 本身就是同比 %，两者同值而不是重复采集（列里只有一个数）
        let yoy_final = yoy.or(Some(v));
        // 一行同时含非制造业指数；不用 let 链（本 crate edition 2021 不支持），
        // 也不用嵌套 if（会被 collapsible_if 抓）
        let non_mfg = if indicator == "pmi" {
            macro_num(r, "NMAKE_INDEX")
        } else {
            None
        };
        if let Some(nm) = non_mfg {
            out.push(MacroDataPoint {
                indicator: "pmi_non_manufacturing".into(),
                display_name: "非制造业采购经理指数(PMI)".into(),
                period: macro_period(r),
                value: nm,
                yoy: None,
                mom: None,
                unit: "%".into(),
                source: macro_source_name(indicator),
                release_date: available.clone(),
            });
        }
        out.push(MacroDataPoint {
            indicator: indicator.into(),
            display_name: display.into(),
            period: macro_period(r),
            value: v,
            yoy: yoy_final,
            mom: None,
            unit: "%".into(),
            source: macro_source_name(indicator),
            release_date: available,
        });
    }
    out
}

/// 取「在 `on_or_before` 当日或之前**已经发布**」的最新统计期。
///
/// 判据是可得日而不是统计期 —— 这一刀就是防前视的那一刀。
fn pick_published(
    points: &[MacroDataPoint],
    on_or_before: chrono::NaiveDate,
) -> Option<MacroDataPoint> {
    points
        .iter()
        .filter(|p| {
            p.release_date
                .as_deref()
                .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
                .is_some_and(|d| d <= on_or_before)
        })
        .max_by_key(|p| p.period.clone())
        .cloned()
}

/// 命名字段落位（indicator 键与 mock 版一致，消费方不需改）。
fn assign_point(snap: &mut MacroDataSnapshot, p: &MacroDataPoint) {
    match p.indicator.as_str() {
        "gdp" => snap.gdp = Some(p.clone()),
        "cpi" => snap.cpi = Some(p.clone()),
        "ppi" => snap.ppi = Some(p.clone()),
        "pmi" => snap.pmi_manufacturing = Some(p.clone()),
        "pmi_non_manufacturing" => snap.pmi_non_manufacturing = Some(p.clone()),
        _ => {},
    }
}

/// 北京时区的「今天」。沿用本模块 2026-07-29 的处理：不用本地时区，
/// 否则非 +8 时区部署会把截止日算错一天，进而错判某期是否已发布。
fn today_cst() -> chrono::NaiveDate {
    let cst = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
    chrono::Utc::now().with_timezone(&cst).date_naive()
}

impl MacroDataClient {
    pub fn new() -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Ok(v) = "https://data.eastmoney.com/cjsj/cpi.html".parse() {
            headers.insert(reqwest::header::REFERER, v);
        }
        let http = reqwest::Client::builder()
            .user_agent(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0 Safari/537.36",
            )
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .unwrap_or_default();
        Self { http }
    }

    async fn fetch_series(&self, report_name: &str) -> Result<Vec<serde_json::Value>, DataError> {
        // `REPORT_DATE` 必须显式倒序：不带排序时返回顺序是乱的（实测），
        // 那样「取最新一期」会变成随机事件。
        let url = format!(
            "https://datacenter-web.eastmoney.com/api/data/v1/get?reportName={report_name}\
             &columns=ALL&pageSize=240&pageNumber=1&source=WEB&client=WEB\
             &sortColumns=REPORT_DATE&sortTypes=-1"
        );
        let resp = self.http.get(&url).send().await?;
        crate::check_response_429(&resp, "macro_eastmoney")?;
        let j: serde_json::Value = resp.json().await?;
        if j.get("success").and_then(serde_json::Value::as_bool) == Some(false) {
            return Err(DataError::VendorError {
                vendor: "macro_eastmoney".into(),
                message: format!(
                    "{report_name}: {}",
                    j.get("message").and_then(|v| v.as_str()).unwrap_or("未知错误")
                ),
            });
        }
        Ok(j.get("result")
            .and_then(|r| r.get("data"))
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    /// 获取宏观经济数据快照。回放模式（as-of）**按发布日**回退到当时已公布的最新期。
    ///
    /// 单条序列失败 ⇒ 该指标保持 `None` 并记一条 `Failure` 级降级（不静默、不用邻近指标顶替）。
    pub async fn snapshot(&self) -> MacroDataSnapshot {
        let as_of = crate::as_of::current_as_of().map(|c| c.as_of_date);
        let on_or_before = as_of.unwrap_or_else(today_cst);
        let mut snap = MacroDataSnapshot::empty(&on_or_before.format("%Y-%m-%d").to_string());
        for (indicator, report_name) in MACRO_SERIES {
            match self.fetch_series(report_name).await {
                Ok(rows) => {
                    let pts = parse_macro_rows(indicator, &rows);
                    match pick_published(&pts, on_or_before) {
                        Some(p) => assign_point(&mut snap, &p),
                        None => {
                            // 该报表有数据但没有一期在截止日前已发布 ⇒ 是「当时尚未公布」，
                            // 不是取数故障 ⇒ 记 NoData 档，面板不会把它算成接口挂了。
                            crate::as_of::record_degradation_kind(
                                "macro_eastmoney",
                                "macro_data_snapshot",
                                &format!(
                                    "宏观指标 {indicator}（{report_name}）在 {on_or_before} 之前无已公布期次"
                                ),
                                crate::as_of::DegradationKind::NoData,
                            );
                        },
                    }
                },
                Err(e) => {
                    tracing::warn!("[macro_data] {report_name} 取数失败: {e}");
                    crate::as_of::record_degradation_kind(
                        "macro_eastmoney",
                        "macro_data_snapshot",
                        &format!("宏观指标 {indicator}（{report_name}）取数失败: {e}"),
                        crate::as_of::DegradationKind::Failure,
                    );
                },
            }
        }
        snap.all = [
            snap.gdp.as_ref(),
            snap.cpi.as_ref(),
            snap.ppi.as_ref(),
            snap.pmi_manufacturing.as_ref(),
            snap.pmi_non_manufacturing.as_ref(),
        ]
        .into_iter()
        .flatten()
        .cloned()
        .collect();
        snap
    }
}

#[cfg(test)]
mod macro_real_source_tests {
    use super::*;
    use chrono::NaiveDate;
    use serde_json::json;

    fn row(report_date: &str, time: &str, cpi_same: f64) -> serde_json::Value {
        json!({ "REPORT_DATE": format!("{report_date} 00:00:00"), "TIME": time, "NATIONAL_SAME": cpi_same,
                "NATIONAL_BASE": 100.0 + cpi_same, "NATIONAL_SEQUENTIAL": 0.4 })
    }

    #[test]
    fn available_at_rules_per_indicator() {
        // CPI/PPI：次月 15 日
        assert_eq!(
            macro_available_at("cpi", "2026-08-01").map(|d| d.format("%Y-%m-%d").to_string()),
            Some("2026-09-15".into())
        );
        // 跨年
        assert_eq!(
            macro_available_at("ppi", "2026-12-01").map(|d| d.format("%Y-%m-%d").to_string()),
            Some("2027-01-15".into())
        );
        // PMI：月末 +2（2 月走闰年判断，2026 非闰 ⇒ 28）
        assert_eq!(
            macro_available_at("pmi", "2026-08-01").map(|d| d.format("%Y-%m-%d").to_string()),
            Some("2026-09-02".into())
        );
        assert_eq!(
            macro_available_at("pmi", "2026-02-01").map(|d| d.format("%Y-%m-%d").to_string()),
            Some("2026-03-02".into())
        );
        // GDP：次月 20 日
        assert_eq!(
            macro_available_at("gdp", "2026-09-01").map(|d| d.format("%Y-%m-%d").to_string()),
            Some("2026-10-20".into())
        );
        // 未知指标 ⇒ 不推断（与「财报非期末不推断」同一条纪律）
        assert_eq!(macro_available_at("m2_yoy", "2026-08-01"), None);
    }

    /// 防前视主证：2026-09 那期在 10-03 尚未公布 ⇒ 回放必须回退到 08。
    #[test]
    fn unpublished_period_is_excluded_and_older_one_selected() {
        // 故意乱序喂进来（实测接口不带排序时就是这个形态）
        let rows = vec![
            row("2026-08-01", "2026年08月份", 0.8),
            row("2026-09-01", "2026年09月份", 0.9),
            row("2026-06-01", "2026年06月份", 0.5),
        ];
        let pts = parse_macro_rows("cpi", &rows);
        let as_of = NaiveDate::from_ymd_opt(2026, 10, 3).unwrap();
        let picked = pick_published(&pts, as_of).expect("至少应有一期已公布");
        assert_eq!(picked.period, "2026年08月份", "09 期可得日是 10-15，10-03 时尚未公布");
        assert_eq!(picked.value, 0.8);
        // 越过发布日 ⇒ 才允许取到 09
        let later = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
        assert_eq!(pick_published(&pts, later).unwrap().period, "2026年09月份");
    }

    #[test]
    fn source_label_declares_the_release_date_is_inferred() {
        let pts = parse_macro_rows("cpi", &[row("2026-08-01", "2026年08月份", 0.8)]);
        assert!(
            pts[0].source.contains("发布日按规则推断"),
            "来源必须自证是推断值: {}",
            pts[0].source
        );
        assert_eq!(pts[0].release_date.as_deref(), Some("2026-09-15"));
    }

    #[test]
    fn cpi_mom_is_dropped_because_the_column_changed_scale_across_years() {
        let pts = parse_macro_rows("cpi", &[row("2026-08-01", "2026年08月份", 0.8)]);
        assert_eq!(pts[0].mom, None, "NATIONAL_SEQUENTIAL 跨年换口径，不得填进 mom");
    }

    #[test]
    fn pmi_row_yields_both_manufacturing_and_non_manufacturing() {
        let rows = vec![json!({
            "REPORT_DATE": "2026-08-01 00:00:00", "TIME": "2026年08月份",
            "MAKE_INDEX": 51.1, "MAKE_SAME": 0.59, "NMAKE_INDEX": 53.2
        })];
        let pts = parse_macro_rows("pmi", &rows);
        let names: Vec<&str> = pts.iter().map(|p| p.indicator.as_str()).collect();
        assert!(names.contains(&"pmi") && names.contains(&"pmi_non_manufacturing"), "{names:?}");
        let mfg = pts.iter().find(|p| p.indicator == "pmi").unwrap();
        assert_eq!(mfg.value, 51.1);
        assert_eq!(mfg.yoy, Some(0.59), "PMI 的同比列与值列不同义，必须分开");
    }
    /// 真接口冒烟（`#[ignore]`，CI 不跑；手动验证：
    /// `cargo test -p axagent-astock-data macro_live_smoke -- --ignored --nocapture`）。
    ///
    /// 只斩取两件事：真的拿到了历史序列、以及 **最新一期不超过已发布范围**。
    /// 后者是防前视的现场判据：取到的 CPI 统计期应至多比今天晚一个月，不得是本月。
    #[tokio::test]
    #[ignore = "需真实网络；仅在手工验证宏观渠道时跑"]
    async fn macro_live_smoke_returns_published_history() {
        let snap = MacroDataClient::new().snapshot().await;
        let today = today_cst();
        for (name, p) in [
            ("cpi", snap.cpi.as_ref()),
            ("ppi", snap.ppi.as_ref()),
            ("pmi", snap.pmi_manufacturing.as_ref()),
            ("gdp", snap.gdp.as_ref()),
        ] {
            let p = p.unwrap_or_else(|| panic!("{name} 未取到——接口形状变了还是被限流了？"));
            let avail = chrono::NaiveDate::parse_from_str(
                p.release_date.as_deref().unwrap_or_default(),
                "%Y-%m-%d",
            )
            .expect("release_date 必须存在且可解析");
            assert!(avail <= today, "{name} 取到了尚未发布的期次: {avail} > {today}");
            assert!(p.source.contains("发布日按规则推断"), "{name} 来源未自证: {}", p.source);
            println!(
                "{name}: period={} value={} yoy={:?} available_at={}",
                p.period, p.value, p.yoy, avail
            );
        }
    }
}
