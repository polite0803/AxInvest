// SPDX-License-Identifier: AGPL-3.0-only
//! 行业成分股工具（工作流侧通道）—— `get_industry_constituents`。
//!
//! # 为什么必须进本注册表
//!
//! 工作流的工具解析（`init/services.rs` 注入的 `ToolResolver`）判据是
//! `reg.list_all_tool_names() ∪ reg.mcp.mcp_tools`。未注册 ⇒ 快速链
//! `t-constituents` ToolNode / 原链聚合注入全部拿到
//! 「工具 'get_industry_constituents' 未注册」（`tool_executor.rs` 的 TOOL_CALL_FAILED）。
//!
//! # 与 Tauri 命令层的关系
//!
//! 实现在 `axagent_analysis_engine::industry_constituents`（implementor 层，有 entities
//! 访问权）；本文件只是薄包装 —— 但**不再直连那个域包**：本 crate 是 consumer，
//! 按依赖铁律不得依赖 implementor（#240 E 批）。数据面改走插件命名命令
//! `stock_get_industry_constituents` → 宿主 `stock.analysis` 接缝，SQL 与归一都留在
//! 域包一侧，本层只做「明显缺料」闸 + 信封组装。
//!
//! # 只读与幂等
//!
//! 纯查询：不落任何表 ⇒ `is_read_only = true`、`is_idempotent = true`。

use crate::tools::plugin_bridge::call_plugin_command;
use crate::{Tool, ToolCategory, ToolContext, ToolDomain, ToolError, ToolResult};
use async_trait::async_trait;
use serde_json::Value;

pub const GET_INDUSTRY_CONSTITUENTS_TOOL: &str = "get_industry_constituents";

/// 插件侧命令名 —— 必须与 `plugin-stock/src/main.rs` 的 `COMMAND_INDUSTRY_CONSTITUENTS`
/// 及插件清单 `commands[].name` **逐字相同**。
const COMMAND: &str = "stock_get_industry_constituents";

/// 插件 id（清单 `name`）。
const PLUGIN_ID: &str = "stock";

pub struct IndustryConstituentsTool;

#[async_trait]
impl Tool for IndustryConstituentsTool {
    fn name(&self) -> &str {
        GET_INDUSTRY_CONSTITUENTS_TOOL
    }

    fn description(&self) -> &str {
        "按行业名查询全部 A 股成分股（确定性，来自本地股票宇宙表）。\
         输入 industries 数组（元素为行业名或 {name, leaderCode}），\
         返回每个行业的成分股分组与匹配模式（exact/normalized/leader_code/missing）。\
         趋势智选候选池的数据面：先取全成分股，再在池内筛选，不要凭训练知识另列股票。"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "industries": {
                    "type": "array",
                    "description": "行业名数组（必填）。元素为字符串，或 {\"name\": 行业名, \
                                    \"leaderCode\": 领涨股代码}（带领涨股时词表落空可反查兜底）。",
                    "items": {
                        "oneOf": [
                            { "type": "string" },
                            {
                                "type": "object",
                                "properties": {
                                    "name": { "type": "string" },
                                    "leaderCode": { "type": "string" }
                                },
                                "required": ["name"]
                            }
                        ]
                    }
                }
            },
            "required": ["industries"]
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

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // 参数错必须显式报错，且不得伪装成「插件命令桥未注入」—— 故本层的闸先于桥。
        // 只闸「明显缺料」：元素形状的宽容解析归插件命令 → 宿主 provider（唯一转换点）。
        let industries = input
            .get("industries")
            .ok_or_else(|| ToolError::invalid_input("缺少 industries 参数"))?;
        let arr = industries.as_array().ok_or_else(|| {
            ToolError::invalid_input("get_industry_constituents industries 必须是数组")
        })?;
        if arr.is_empty() {
            return Err(ToolError::invalid_input(
                "get_industry_constituents industries 为空：至少给一个行业名",
            ));
        }

        let report = call_plugin_command(ctx, PLUGIN_ID, COMMAND, input.clone())
            .await
            .map_err(|e| ToolError::execution_failed(format!("行业成分股查询失败: {e}")))?;
        // 出参信封 `{status, data, detail}` 是**工具面契约**：rhai 消费端
        // （candidate-pool.rhai 的 to_array）按信封下钻 data。接缝只给域 DTO
        // （`{groups, universeTotal, universeWithIndustry}`），信封在本层组装。
        let groups = report.get("groups").cloned().unwrap_or(Value::Array(Vec::new()));
        let total = report.get("universeTotal").map(value_as_u64).unwrap_or(0);
        let with_industry = report.get("universeWithIndustry").map(value_as_u64).unwrap_or(0);
        let ok = serde_json::json!({
            "status": "ok",
            "data": groups,
            "detail": format!(
                "universe_total={total} universe_with_industry={with_industry}"
            ),
        });
        Ok(ToolResult::success(ok.to_string()))
    }
}

/// 接缝给的是 JSON 数字；`u64` 与 `f64` 两种编码都要吃下（serde 对 `u64` 溢出会降级成 f64）。
fn value_as_u64(v: &Value) -> u64 {
    v.as_u64().or_else(|| v.as_f64().map(|f| f.max(0.0) as u64)).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_name_and_readonly_flags() {
        let t = IndustryConstituentsTool;
        assert_eq!(t.name(), "get_industry_constituents");
        assert!(t.is_read_only());
        assert!(t.is_idempotent());
        assert_eq!(t.domain(), ToolDomain::Finance);
    }

    #[test]
    fn empty_industries_rejected_before_bridge_call() {
        // 参数错必须显式报错，且不得伪装成「插件命令桥未注入」（闸先于桥）。
        let t = IndustryConstituentsTool;
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let out =
            rt.block_on(t.call(serde_json::json!({"industries": []}), &ToolContext::new(".")));
        let err = out.expect_err("空 industries 应报错");
        assert!(err.to_string().contains("industries 为空"), "实际: {err}");
        assert!(!err.to_string().contains("插件命令桥未注入"), "闸必须早于桥：{err}");
    }

    #[test]
    fn non_array_industries_rejected_before_bridge_call() {
        let t = IndustryConstituentsTool;
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let out =
            rt.block_on(t.call(serde_json::json!({"industries": 42}), &ToolContext::new(".")));
        let err = out.expect_err("坏参数应报错");
        assert!(err.to_string().contains("必须是数组"), "实际: {err}");
    }

    /// 桥未注入 ⇒ fail-closed（不回落直连 `analysis-engine`）。
    #[test]
    fn missing_bridge_fails_closed() {
        let t = IndustryConstituentsTool;
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let err = rt
            .block_on(t.call(serde_json::json!({"industries": ["半导体"]}), &ToolContext::new(".")))
            .expect_err("桥缺失必须失败");
        assert!(err.to_string().contains("插件命令桥未注入"), "实际: {err}");
    }

    #[test]
    fn passthrough_and_envelope_are_verbatim() {
        // 出参信封形状是**工具面契约**（rhai 按 status/data/detail 下钻），钉死。
        #[derive(Debug)]
        struct Stub(Value);
        #[async_trait]
        impl axagent_harness::PluginCommandBridge for Stub {
            async fn execute(
                &self,
                plugin_id: &str,
                command: &str,
                input: Value,
            ) -> Result<Value, String> {
                assert_eq!(plugin_id, "stock");
                assert_eq!(command, "stock_get_industry_constituents");
                // 入参**逐字**进桥（工具层不改名、不裁剪）。
                assert_eq!(input, serde_json::json!({"industries": ["半导体"]}));
                Ok(self.0.clone())
            }
        }
        let t = IndustryConstituentsTool;
        let ctx = ToolContext::new("mock").with_plugin_command_bridge(std::sync::Arc::new(Stub(
            serde_json::json!({
                "groups": [{"industry": "半导体", "matchedIndustry": null,
                            "matchMode": "missing", "stocks": []}],
                "universeTotal": 5123,
                "universeWithIndustry": 3700,
            }),
        )));
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let out = rt
            .block_on(t.call(serde_json::json!({"industries": ["半导体"]}), &ctx))
            .expect("桥桩应答应成功");
        let payload: Value = serde_json::from_str(&out.content).expect("信封必须是 JSON 文本");
        assert_eq!(payload["status"], "ok");
        assert_eq!(payload["data"][0]["industry"], "半导体");
        assert_eq!(payload["detail"], "universe_total=5123 universe_with_industry=3700");
    }
}
