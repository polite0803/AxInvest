// SPDX-License-Identifier: AGPL-3.0-only
//! 逐票 1 月涨跌幅取数（阶段 3「打分重构」的因子数据面）。
//!
//! # 解决什么
//!
//! 阶段 1 全市场因子 IC 普查（见 `PLAN-serenity-full-coverage-predictive.md` §阶段1）的
//! 唯一稳定有效因子是 **1 月短期反转 rev_1m**（过去一月跌得多的票之后跑赢，
//! mean IC +0.060~+0.066，NW 校正后 5/10/20 日视野显著）。本模块把该因子做成
//! 确定性取数：对给定代码清单，从 `market_daily_close` 计算每只票截至截止日的
//! 1 月涨跌幅（21 个交易日间隔），供打分脚本做横截面反转排序。
//!
//! # as-of 语义
//!
//! `as_of_date` 缺省 = 不加日期过滤（取每只票自身序列的最近 22 行）；
//! 给定 = 只用 `trade_date <= as_of_date` 的行（ISO 日期字典序即时间序）。
//! 样本外回测（阶段 4）按决策日传入；live 链缺省取最新。
//!
//! # 输入形态（宽容归一）
//!
//! 调用方两条链注入的形态不同：快速链传池对象数组（`[{code,name,…}]`），
//! 原链传 `get_industry_constituents` 的信封（`{status,data:[{stocks:[…]}]}`）。
//! `parse_stock_refs` 统一归一成 6 位代码清单，归一不出代码的元素跳过并计入 requested
//! 对照（缺席如实呈现，不静默丢弃后伪装成「全部打过分」）。
//!
//! # 分层
//!
//! implementor（依赖 entities）⇒ DB 访问合法；工作流通达由 tools crate 薄包装
//! `get_stock_factor` + `register_all` 注册完成（同 `market_mainline` / 阶段 2 的
//! `get_industry_constituents` 先例）。

use axagent_entities::market_daily_close;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

/// 1 月 = 21 个交易日间隔 ⇒ 需要 22 行收盘（首尾各一）。
pub const WINDOW_ROWS: usize = 22;

/// 单票因子行（输出 camelCase，对齐全站 DTO 规范）。
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StockFactorRow {
    pub stock_code: String,
    /// 1 月涨跌幅（小数，+0.10 = 涨 10%）。方向语义（反转）由打分脚本负责。
    pub chg_1m: f64,
    /// 截止日（序列最后一行的 trade_date）
    pub as_of_date: String,
    /// 基期日（21 个交易日前那一行的 trade_date）
    pub base_date: String,
}

/// 因子报告（缺席显式：`missing` 是「要了但算不出」的代码清单，不是静默差集）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StockFactorReport {
    pub rows: Vec<StockFactorRow>,
    pub requested: usize,
    pub scored: usize,
    pub missing: Vec<String>,
    /// 实际使用的截止日（各票可能不同——停牌票的最后一行更早），取全体最大值供展示
    pub cutoff_date: String,
    pub window_rows: usize,
}

/// 从收盘序列算 1 月涨跌幅：`last / first - 1`，first = 倒数第 22 行。
/// 序列长度不足 / 有非有限值 / 基期非正 ⇒ None（调用方计缺席）。
pub fn chg_1m_from_closes(closes: &[f64]) -> Option<f64> {
    if closes.len() < WINDOW_ROWS {
        return None;
    }
    let first = closes[closes.len() - WINDOW_ROWS];
    let last = closes[closes.len() - 1];
    if !first.is_finite() || !last.is_finite() || first <= 0.0 {
        return None;
    }
    Some(last / first - 1.0)
}

/// 从 flexible 形态归一出代码清单（保序去重，trim 后整串取用）。
/// 支持元素：`"000001"` 字符串 / `{"stockCode"|"stock_code"|"code": "…"}`
/// / 含 `stocks` 数组的分组对象；顶层：数组 / `{"data": …}` 信封 / `{"arguments": …}`。
pub fn parse_stock_refs(v: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = Default::default();
    collect_refs(v, &mut out, &mut seen);
    out
}

fn first_code_of(obj: &serde_json::Map<String, Value>) -> Option<String> {
    for key in ["stockCode", "stock_code", "code"] {
        if let Some(Value::String(s)) = obj.get(key) {
            let t = s.trim().to_string();
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    None
}

fn collect_refs(v: &Value, out: &mut Vec<String>, seen: &mut std::collections::HashSet<String>) {
    match v {
        Value::Array(items) => {
            for it in items {
                collect_refs(it, out, seen);
            }
        },
        Value::Object(obj) => {
            if let Some(stocks) = obj.get("stocks") {
                collect_refs(stocks, out, seen);
                return;
            }
            if let Some(inner) = obj.get("data").or_else(|| obj.get("arguments")) {
                collect_refs(inner, out, seen);
                return;
            }
            if let Some(code) = first_code_of(obj)
                && seen.insert(code.clone())
            {
                out.push(code);
            }
        },
        Value::String(s) => {
            let t = s.trim().to_string();
            if !t.is_empty() && seen.insert(t.clone()) {
                out.push(t);
            }
        },
        _ => {},
    }
}

/// 逐票计算截至 `as_of`（None = 各票自身序列最新）的 1 月涨跌幅。
pub async fn fetch_stock_chg_1m(
    db: &DatabaseConnection,
    codes: &[String],
    as_of: Option<&str>,
) -> Result<StockFactorReport, sea_orm::DbErr> {
    let cutoff = as_of.unwrap_or("").to_string();
    let mut query = market_daily_close::Entity::find()
        .filter(market_daily_close::Column::StockCode.is_in(codes.to_vec()))
        .order_by_asc(market_daily_close::Column::StockCode)
        .order_by_asc(market_daily_close::Column::TradeDate);
    if !cutoff.is_empty() {
        // trade_date 是 ISO 字符串，字典序 = 时间序
        query = query.filter(market_daily_close::Column::TradeDate.lte(&cutoff));
    }
    let rows = query.all(db).await?;

    // 按代码聚合并保序（查询已按 code+date 排序）
    let mut series: BTreeMap<String, Vec<(String, f64)>> = Default::default();
    for r in rows {
        series.entry(r.stock_code.clone()).or_default().push((r.trade_date, r.close));
    }

    let mut out_rows = Vec::new();
    let mut missing = Vec::new();
    let mut max_cutoff = String::new();
    for code in codes {
        match series.get(code) {
            Some(points) => {
                let closes: Vec<f64> = points.iter().map(|(_, c)| *c).collect();
                match chg_1m_from_closes(&closes) {
                    Some(chg) => {
                        let n = points.len();
                        let as_of_date = points[n - 1].0.clone();
                        let base_date = points[n - WINDOW_ROWS].0.clone();
                        if as_of_date > max_cutoff {
                            max_cutoff = as_of_date.clone();
                        }
                        out_rows.push(StockFactorRow {
                            stock_code: code.clone(),
                            chg_1m: chg,
                            as_of_date,
                            base_date,
                        });
                    },
                    None => missing.push(code.clone()),
                }
            },
            None => missing.push(code.clone()),
        }
    }
    // 单票停牌导致截止日偏旧时不夸大：cutoff_date 只在拿到行时才有值
    if max_cutoff.is_empty() {
        max_cutoff = cutoff;
    }
    Ok(StockFactorReport {
        scored: out_rows.len(),
        rows: out_rows,
        requested: codes.len(),
        missing,
        cutoff_date: max_cutoff,
        window_rows: WINDOW_ROWS,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn chg_1m_needs_full_window_and_positive_base() {
        assert_eq!(chg_1m_from_closes(&[1.0; 21]), None, "不足 22 行");
        assert_eq!(chg_1m_from_closes(&[1.0; 22]), Some(0.0), "22 行横盘 = 0");
        let mut v = vec![1.0; 22];
        v[21] = 1.10;
        assert!((chg_1m_from_closes(&v).unwrap() - 0.10).abs() < 1e-12);
        let mut bad = vec![1.0; 22];
        bad[0] = 0.0;
        assert_eq!(chg_1m_from_closes(&bad), None, "基期非正");
        let mut nan = vec![1.0; 22];
        nan[21] = f64::NAN;
        assert_eq!(chg_1m_from_closes(&nan), None, "非有限值");
    }

    #[test]
    fn parse_refs_accepts_all_chain_shapes() {
        // 原链形态：get_industry_constituents 信封（分组嵌套 stocks）
        let envelope = json!({"status": "ok", "data": [
            {"industry": "银行", "stocks": [{"stockCode": "000001"}, {"stockCode": "600036"}]},
            {"industry": "券商", "stocks": [{"stockCode": "300059"}]}
        ]});
        assert_eq!(parse_stock_refs(&envelope), vec!["000001", "600036", "300059"]);
        // 快速链形态：池对象数组（code 键）
        let pool = json!([{"code": "002049", "name": "紫光国微"}, "600519"]);
        assert_eq!(parse_stock_refs(&pool), vec!["002049", "600519"]);
        // arguments 包装 + 去重保序
        let wrapped = json!({"arguments": [{"code": "000001"}, {"stock_code": "600036"}, {"code": "000001"}]});
        assert_eq!(parse_stock_refs(&wrapped), vec!["000001", "600036"]);
        // 全归一不出 ⇒ 空清单（调用方报「无可打分代码」，不伪装成功）
        assert_eq!(parse_stock_refs(&json!({"status": "ok", "data": []})), Vec::<String>::new());
        assert_eq!(parse_stock_refs(&json!(42)), Vec::<String>::new());
    }

    #[test]
    fn report_serializes_camel_case() {
        let r = StockFactorReport {
            rows: vec![StockFactorRow {
                stock_code: "000001".into(),
                chg_1m: -0.12,
                as_of_date: "2026-10-30".into(),
                base_date: "2026-09-28".into(),
            }],
            requested: 2,
            scored: 1,
            missing: vec!["600000".into()],
            cutoff_date: "2026-10-30".into(),
            window_rows: WINDOW_ROWS,
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(
            s.contains("\"stockCode\"") && s.contains("\"chg1m\"") && s.contains("\"asOfDate\"")
        );
        assert!(s.contains("\"cutoffDate\"") && s.contains("\"windowRows\""));
    }
}
