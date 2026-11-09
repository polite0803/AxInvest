// SPDX-License-Identifier: AGPL-3.0-only
//! `get_stock_factor` — 逐票 1 月反转因子查询（阶段 3 打分重构的数据面）。
//!
//! 数据面走插件命名命令 `stock_get_stock_factor` → 宿主 `stock.analysis` 接缝
//! （#240 E 批）：本 crate 是 consumer，不得依赖 `analysis-engine`（implementor）。
//!
//! # 参数契约（系统边界，宽容归一但缺料显式报错）
//!
//! - `stock_codes`（必填）：宽容形态——字符串码 / `{"stockCode"|"stock_code"|"code": …}`
//!   对象数组 / 含 `stocks` 分组的信封对象。**归一在宿主 provider**（唯一转换点，
//!   `parse_stock_refs` 就在那里）；本层只闸「给了键但明显归不出票」的形态。
//!   归一后为空 ⇒ provider 显式报错。
//! - `as_of_date`（可选，ISO 字符串）：样本外回测按决策日传；缺省 = 各票序列最新。
//!
//! # 输出信封（**工具面契约，不得改形**）
//!
//! `{"status":"ok","data":rows,"detail":"requested=N scored=M missing=K cutoff=…","missing":[…]}}`
//! `data` = `[{stockCode, chg1m, asOfDate, baseDate}]`；缺席票不在 rows 里
//! （打分脚本按「score_map 无此键」判缺席并如实呈现）。

use crate::tools::plugin_bridge::call_plugin_command;
use crate::{Tool, ToolCategory, ToolContext, ToolDomain, ToolError, ToolResult};
use serde_json::Value;

pub const GET_STOCK_FACTOR_TOOL: &str = "get_stock_factor";

/// 插件侧命令名 —— 必须与 `plugin-stock/src/main.rs` 的 `COMMAND_STOCK_FACTOR`
/// 及插件清单 `commands[].name` **逐字相同**。
const COMMAND: &str = "stock_get_stock_factor";

/// 插件 id（清单 `name`）。
const PLUGIN_ID: &str = "stock";

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

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let refs = args
            .get("stock_codes")
            .ok_or_else(|| ToolError::invalid_input("缺少 stock_codes 参数"))?;
        // 只闸「明显为空」的形态（null / 空数组 / `stocks: []`）：这类请求送到插件
        // 只会换回同一条 Validation，白跑一次命令帧。真正的宽容归一（对象数组、信封、
        // 代码清洗）归宿主 provider —— 那才是唯一转换点。
        if obviously_empty(refs) {
            return Err(ToolError::invalid_input(
                "stock_codes 归一后为空：无法打分（缺席要显式报错，不得伪装成空结果）",
            ));
        }

        let report = call_plugin_command(ctx, PLUGIN_ID, COMMAND, args.clone())
            .await
            .map_err(|e| ToolError::execution_failed(format!("因子查询失败: {e}")))?;
        // 信封在本层组装（工具面契约）：接缝给的是域报告
        // `{rows, requested, scored, missing, cutoffDate, windowRows}`。
        let ok = serde_json::json!({
            "status": "ok",
            "data": report.get("rows").cloned().unwrap_or(Value::Array(Vec::new())),
            "detail": format!(
                "requested={} scored={} missing={} cutoff={} windowRows={}",
                field_u64(&report, "requested"),
                field_u64(&report, "scored"),
                report.get("missing").and_then(Value::as_array).map_or(0, Vec::len),
                report.get("cutoffDate").and_then(Value::as_str).unwrap_or(""),
                field_u64(&report, "windowRows"),
            ),
            "missing": report.get("missing").cloned().unwrap_or(Value::Array(Vec::new())),
        });
        Ok(ToolResult::success(ok.to_string()))
    }
}

/// 「明显为空」：`null`、空数组、或 `{stocks: []}` / `{items: []}` 这类已知分组键为空。
///
/// 故意**不**判「归一后确实无票」—— 那是 `parse_stock_refs` 的职责，在 provider 侧。
fn obviously_empty(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::Array(a) => a.is_empty(),
        Value::Object(m) => {
            ["stocks", "items", "data"].iter().filter_map(|k| m.get(*k)).any(obviously_empty)
                || m.is_empty()
        },
        _ => false,
    }
}

fn field_u64(report: &Value, key: &str) -> u64 {
    report
        .get(key)
        .and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f.max(0.0) as u64)))
        .unwrap_or(0)
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

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().build().unwrap()
    }

    #[test]
    fn empty_refs_rejected_before_bridge_call() {
        // 参数缺料必须显式报错，且不得伪装成「插件命令桥未注入」（闸先于桥）。
        let t = StockFactorTool;
        let err = rt()
            .block_on(t.call(serde_json::json!({"stock_codes": []}), &ToolContext::new(".")))
            .expect_err("空清单应报错");
        assert!(err.to_string().contains("归一后为空"), "实际: {err}");
        assert!(!err.to_string().contains("插件命令桥未注入"), "闸必须早于桥：{err}");
    }

    /// 信封形态的空分组（`{stocks: []}`）也算明显缺料 —— 差旅模板就是这么包的。
    #[test]
    fn empty_group_envelope_rejected() {
        let t = StockFactorTool;
        let err = rt()
            .block_on(
                t.call(serde_json::json!({"stock_codes": {"stocks": []}}), &ToolContext::new(".")),
            )
            .expect_err("空分组应报错");
        assert!(err.to_string().contains("归一后为空"), "实际: {err}");
    }

    /// 桥未注入 ⇒ fail-closed（不回落直连 `analysis-engine`）。
    #[test]
    fn missing_bridge_fails_closed() {
        let t = StockFactorTool;
        let err = rt()
            .block_on(
                t.call(serde_json::json!({"stock_codes": ["600519"]}), &ToolContext::new(".")),
            )
            .expect_err("桥缺失必须失败");
        assert!(err.to_string().contains("插件命令桥未注入"), "实际: {err}");
    }

    /// 工具层**不**做宽容归一：含不可识别键的对象照样送进桥，由 provider 判
    /// （判据就是「到了桥」而不是「本地提前报空」）。
    #[test]
    fn unparsable_shape_still_goes_to_bridge() {
        let t = StockFactorTool;
        let err = rt()
            .block_on(t.call(serde_json::json!({"stock_codes": {"x": 1}}), &ToolContext::new(".")))
            .expect_err("无桥必须失败");
        assert!(
            err.to_string().contains("插件命令桥未注入"),
            "不得在工具层提前判空（归一归 provider）：{err}"
        );
    }

    #[test]
    fn passthrough_and_envelope_are_verbatim() {
        #[derive(Debug)]
        struct Stub;
        #[async_trait::async_trait]
        impl axagent_harness::PluginCommandBridge for Stub {
            async fn execute(
                &self,
                plugin_id: &str,
                command: &str,
                input: Value,
            ) -> Result<Value, String> {
                assert_eq!(plugin_id, "stock");
                assert_eq!(command, "stock_get_stock_factor");
                // 宽容形态**原样**进桥（工具层不改名、不裁剪、不归一）。
                assert_eq!(
                    input,
                    serde_json::json!({"stock_codes": ["600519", "000001"], "as_of_date": "2026-09-30"})
                );
                Ok(serde_json::json!({
                    "rows": [
                        {"stockCode": "600519", "chg1m": 0.052, "asOfDate": "2026-09-30",
                         "baseDate": "2026-08-28"}
                    ],
                    "requested": 2,
                    "scored": 1,
                    "missing": ["000001"],
                    "cutoffDate": "2026-09-30",
                    "windowRows": 22,
                }))
            }
        }
        let t = StockFactorTool;
        let ctx = ToolContext::new("mock").with_plugin_command_bridge(std::sync::Arc::new(Stub));
        let out = rt()
            .block_on(t.call(
                serde_json::json!({"stock_codes": ["600519", "000001"], "as_of_date": "2026-09-30"}),
                &ctx,
            ))
            .expect("桥桩应答应成功");
        let payload: Value = serde_json::from_str(&out.content).expect("信封必须是 JSON 文本");
        assert_eq!(payload["status"], "ok");
        assert_eq!(payload["data"][0]["stockCode"], "600519");
        assert_eq!(payload["data"][0]["chg1m"], 0.052);
        assert_eq!(payload["missing"][0], "000001");
        assert_eq!(
            payload["detail"],
            "requested=2 scored=1 missing=1 cutoff=2026-09-30 windowRows=22"
        );
    }
}
