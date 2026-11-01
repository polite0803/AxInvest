// SPDX-License-Identifier: AGPL-3.0-only
//! `get_stock_factor` — 逐票 1 月反转因子查询（阶段 3 打分重构的数据面）。
//!
//! 薄包装先例同 `industry_constituents.rs`：实现权威在 analysis-engine（implementor，
//! 有 entities），本层只做参数校验 + `global_state::get_sea_db()` + 信封输出。
//!
//! # 参数契约（系统边界，宽容归一但缺料显式报错）
//!
//! - `stock_codes`（必填）：宽容形态——字符串码 / `{"stockCode"|"stock_code"|"code": …}`
//!   对象数组 / 含 `stocks` 分组的信封对象（见 `analysis_engine::stock_factor::parse_stock_refs`）。
//!   归一后为空 ⇒ 在取 DB **之前**显式报错。
//! - `as_of_date`（可选，ISO 字符串）：样本外回测按决策日传；缺省 = 各票序列最新。
//!
//! # 输出信封
//!
//! `{"status":"ok","data":rows,"detail":"requested=N scored=M missing=K cutoff=…"}`
//! `data` = `[{stockCode, chg1m, asOfDate, baseDate}]`；缺席票不在 rows 里
//! （打分脚本按「score_map 无此键」判缺席并如实呈现）。

use crate::{Tool, ToolCategory, ToolContext, ToolDomain, ToolError, ToolResult, global_state};
use axagent_analysis_engine::stock_factor::{
    StockFactorReport, fetch_stock_chg_1m, parse_stock_refs,
};
use serde_json::Value;

pub const GET_STOCK_FACTOR_TOOL: &str = "get_stock_factor";

pub struct StockFactorTool;

#[async_trait::async_trait]
impl Tool for StockFactorTool {
    fn name(&self) -> &str {
        GET_STOCK_FACTOR_TOOL
    }

    fn description(&self) -> &str {
        "逐票查询 1 月涨跌幅（rev_1m 反转因子原料）：输入股票代码清单（宽容形态），\
         返回每只票截至截止日的 1 月涨跌幅与基期日；缺席票单独计数"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "stock_codes": {
                    "description": "股票代码清单（必填，宽容形态）：字符串数组、\
                     {\"stockCode\"|\"code\"} 对象数组、或含 stocks 分组的信封对象",
                },
                "as_of_date": {
                    "type": "string",
                    "description": "可选截止日（YYYY-MM-DD）；缺省 = 各票序列最新"
                }
            },
            "required": ["stock_codes"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Finance
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn is_idempotent(&self) -> bool {
        true
    }

    async fn call(
        &self,
        args: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let refs = args
            .get("stock_codes")
            .ok_or_else(|| ToolError::invalid_input("缺少 stock_codes 参数"))?;
        let codes = parse_stock_refs(refs);
        if codes.is_empty() {
            return Err(ToolError::invalid_input(
                "stock_codes 归一后为空：无法打分（缺席要显式报错，不得伪装成空结果）",
            ));
        }
        let as_of = args
            .get("as_of_date")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string);

        let db = global_state::get_sea_db().ok_or_else(|| {
            ToolError::execution_failed("数据库未初始化（wiring 缺失：init 须调用 set_sea_db）")
        })?;
        let report: StockFactorReport = fetch_stock_chg_1m(db.as_ref(), &codes, as_of.as_deref())
            .await
            .map_err(|e| ToolError::execution_failed(format!("因子查询失败: {e}")))?;
        let ok = serde_json::json!({
            "status": "ok",
            "data": serde_json::to_value(&report.rows)
                .map_err(|e| ToolError::execution_failed(format!("序列化失败: {e}")))?,
            "detail": format!(
                "requested={} scored={} missing={} cutoff={} windowRows={}",
                report.requested, report.scored, report.missing.len(),
                report.cutoff_date, report.window_rows
            ),
            "missing": report.missing,
        });
        Ok(ToolResult::success(ok.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_name_and_flags() {
        let t = StockFactorTool;
        assert_eq!(t.name(), "get_stock_factor");
        assert!(t.is_read_only());
        assert!(t.is_idempotent());
        assert_eq!(t.domain(), ToolDomain::Finance);
    }

    #[test]
    fn empty_refs_rejected_before_db_lookup() {
        // 参数缺料必须显式报错，且不得伪装成「数据库未初始化」。
        let t = StockFactorTool;
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let err = rt
            .block_on(t.call(serde_json::json!({"stock_codes": []}), &ToolContext::new(".")))
            .expect_err("空清单应报错");
        assert!(err.to_string().contains("归一后为空"), "实际: {err}");
    }

    #[test]
    fn malformed_params_rejected_before_db_lookup() {
        let t = StockFactorTool;
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let err = rt
            .block_on(t.call(serde_json::json!({"stock_codes": {"x": 1}}), &ToolContext::new(".")))
            .expect_err("归一不出代码应报错");
        assert!(err.to_string().contains("归一后为空"), "实际: {err}");
    }
}
