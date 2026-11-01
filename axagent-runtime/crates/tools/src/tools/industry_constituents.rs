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
//! 访问权）；本文件只是薄包装，不自己写 SQL。DB 经 `global_state::get_sea_db()`
//! 槽位（`init/database.rs` 启动时注入）—— 同 `market_mainline.rs` 先例。
//!
//! # 只读与幂等
//!
//! 纯查询：不落任何表 ⇒ `is_read_only = true`、`is_idempotent = true`。

use crate::{Tool, ToolCategory, ToolContext, ToolDomain, ToolError, ToolResult, global_state};
use async_trait::async_trait;
use axagent_analysis_engine::industry_constituents::{
    IndustryConstituentsInput, fetch_industry_constituents,
};
use serde_json::Value;

pub const GET_INDUSTRY_CONSTITUENTS_TOOL: &str = "get_industry_constituents";

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

    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // 先解析参数、再取 DB：否则「参数错」会被误报成「数据库未初始化」。
        let parsed: IndustryConstituentsInput = serde_json::from_value(input).map_err(|e| {
            ToolError::invalid_input(format!(
                "get_industry_constituents 参数解析失败: {e}。期望 industries 数组\
                 （元素为行业名字符串或 {{name, leaderCode}}）"
            ))
        })?;
        if parsed.industries.is_empty() {
            return Err(ToolError::invalid_input(
                "get_industry_constituents industries 为空：至少给一个行业名",
            ));
        }
        let db = global_state::get_sea_db().ok_or_else(|| {
            ToolError::execution_failed("数据库未初始化（wiring 缺失：init 须调用 set_sea_db）")
        })?;
        let report = fetch_industry_constituents(db.as_ref(), &parsed)
            .await
            .map_err(|e| ToolError::execution_failed(format!("行业成分股查询失败: {e}")))?;
        // 输出走 {status, data, detail} 信封 —— 与 astock 工具族一致，
        // rhai 消费端（candidate-pool.rhai 的 to_array）按信封下钻 data。
        let ok = serde_json::json!({
            "status": "ok",
            "data": serde_json::to_value(&report.groups).unwrap_or_default(),
            "detail": format!(
                "universe_total={} universe_with_industry={}",
                report.universe_total, report.universe_with_industry
            ),
        });
        Ok(ToolResult::success(ok.to_string()))
    }
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
    fn empty_industries_rejected_before_db_lookup() {
        // 参数错必须显式报错，且不得伪装成「数据库未初始化」。
        let t = IndustryConstituentsTool;
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let out =
            rt.block_on(t.call(serde_json::json!({"industries": []}), &ToolContext::new(".")));
        let err = out.expect_err("空 industries 应报错");
        assert!(err.to_string().contains("industries 为空"), "实际: {err}");
    }

    #[test]
    fn malformed_params_rejected_before_db_lookup() {
        let t = IndustryConstituentsTool;
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let out =
            rt.block_on(t.call(serde_json::json!({"industries": 42}), &ToolContext::new(".")));
        let err = out.expect_err("坏参数应报错");
        assert!(err.to_string().contains("参数解析失败"), "实际: {err}");
    }
}
