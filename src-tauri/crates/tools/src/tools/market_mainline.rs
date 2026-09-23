// SPDX-License-Identifier: AGPL-3.0-only
//! 市场主线工具（**工作流侧**通道）
//!
//! # 为什么必须存在本文件
//!
//! `daily-market-events` 工作流模板的 Agent 节点在自己的 `agent_tools` 里声明了
//! `ToolDef { name: "market_mainline_batch_upsert" }`，并在提示词里要求模型
//! 「调用 `market_mainline_batch_upsert` 工具持久化结果（archive_missing=true）」。
//!
//! 但工作流的工具解析**不走** Tauri 命令层：`init/services.rs` 注入的 `ToolResolver`
//! 的判据是
//!
//! ```text
//! known = reg.list_all_tool_names().contains(name) || reg.mcp.mcp_tools.contains_key(name)
//! ```
//!
//! 即「`axagent_tools::tools::register_all` 注册的工具集 ∪ MCP 工具表」。
//! `#[agent_command]` 元数据**不在这两个集合里**（它只喂 `command_bridge` 的
//! `execute_tauri_command` 索引）。所以本文件缺失时，`WorkEngine::run_workflow`
//! 的注册段会打
//!
//! ```text
//! [WorkEngine] 工具 'market_mainline_batch_upsert' 在注册表中未找到
//! ```
//!
//! 然后该工具的 handler 永不注册 ⇒ 模型调用拿到「工具未注册」⇒ 被 `core.rs` 的
//! Failed 分支 `emit degraded: true` **静默吞掉**（节点仍 `completed`、结果为空）。
//! 这正是 `seed_consistency_tests.rs` 顶部注释描述的「最隐蔽的故障形态」。
//!
//! 同型先例见 `tools/mod.rs` 的 OPC 段（27 个 OPC 工具此前只用于产出 ToolDef schema、
//! 从未进注册表，补注册后 resolver 的 `known` 分支即刻接通）。
//!
//! # 与 Tauri 命令层的关系
//!
//! **同一份 crate 实现**（`axagent_analysis_engine::market_mainline`），两处都不自己写 SQL：
//! - 本条通道：工作流 AgentNode / chat 工具调用
//! - `src/commands/market_mainline.rs`：前端 `invoke` 与 chat 的 `execute_tauri_command`
//!
//! # 参数契约
//!
//! `BatchUpsertInput` 的 serde 主名是 camelCase，但提示词示例（`market-mainline/SKILL.md`、
//! `market-synthesizer.md`）是 snake_case ⇒ 该 DTO 已对每个多词字段补 `alias`，
//! **两种写法都收**。`archive_missing` 尤甚：漏收会让「归档当日未提及主线」静默不执行。

use crate::{Tool, ToolCategory, ToolContext, ToolDomain, ToolError, ToolResult};
use async_trait::async_trait;
use axagent_analysis_engine::market_mainline::{BatchUpsertInput, batch_upsert_mainlines};
use serde_json::Value;

/// 工具名 —— 必须与 `seed_daily_market_events.rs` 的 `ToolDef.name` **逐字相同**。
/// 改这里必须同步改模板（并递增 `TEMPLATE_VERSION` 触发重种子），否则模型调的还是旧名。
pub const MARKET_MAINLINE_BATCH_UPSERT_TOOL: &str = "market_mainline_batch_upsert";

/// 批量 upsert 市场主线到 `market_mainlines` 表。
pub struct MarketMainlineBatchUpsertTool;

#[async_trait]
impl Tool for MarketMainlineBatchUpsertTool {
    fn name(&self) -> &str {
        MARKET_MAINLINE_BATCH_UPSERT_TOOL
    }

    fn description(&self) -> &str {
        "批量写入市场主线（同日同主题更新，archive_missing=true 时归档当日未提及的主线）。\
         返回 inserted / updated / archived 三个计数。"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": true,
            "properties": {
                "mainline_date": {
                    "type": "string",
                    "description": "主线日期 YYYY-MM-DD（必填）。也接受 camelCase 的 mainlineDate。"
                },
                "mainlines": {
                    "type": "array",
                    "description": "主线数组（必填）。每项含 theme / theme_category / narrative / \
                                    representative_symbols / strength_score / persistence / evidence。",
                    "items": {
                        "type": "object",
                        "additionalProperties": true,
                        "properties": {
                            "theme": { "type": "string", "description": "主题名，2-6 字（必填）" },
                            "theme_category": {
                                "type": "string",
                                "description": "主题大类：科技/消费/周期/金融/医药/政策/其他，缺省「其他」"
                            },
                            "narrative": { "type": "string", "description": "1-2 句故事线（必填）" },
                            "representative_symbols": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "代表性标的（6 位 A 股代码），缺省空数组"
                            },
                            "strength_score": {
                                "type": "number",
                                "description": "强度评分 0-100，缺省 0"
                            },
                            "persistence": {
                                "type": "string",
                                "description": "持续性判断 1d/1w/1m/fading/emerging，缺省 1d"
                            },
                            "evidence": { "type": "object", "description": "证据 JSON，缺省空对象" }
                        },
                        "required": ["theme", "narrative"]
                    }
                },
                "archive_missing": {
                    "type": "boolean",
                    "description": "是否把当日已有但本次未提及的主线置为 archived。缺省 false。\
                                    也接受 camelCase 的 archiveMissing。"
                },
                "source_workflow_execution_id": {
                    "type": "string",
                    "description": "来源工作流执行 ID（可省略）"
                }
            },
            "required": ["mainline_date", "mainlines"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Finance
    }

    /// 按 (mainline_date, theme) upsert：同一输入重复执行得到同一状态 ⇒ 幂等。
    fn is_idempotent(&self) -> bool {
        true
    }

    /// 会写 `market_mainlines` 表 ⇒ 非只读。
    fn is_read_only(&self) -> bool {
        false
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // 先解析参数、再取 DB：否则「参数错」会被误报成「数据库未初始化」。
        //
        // 解析失败必须**显式报错**，不能回落到默认值 —— 本 DTO 的字段多为
        // `#[serde(default)]`，一旦把 unknown/missing 当成"用默认值"，就会写出
        // strength_score=0、representative_symbols=[] 的假主线（"成功写入、数据全错"）。
        let params: BatchUpsertInput = serde_json::from_value(input).map_err(|e| {
            ToolError::invalid_input(format!(
                "market_mainline_batch_upsert 参数解析失败: {e}。\
                 期望字段: mainline_date(YYYY-MM-DD) / mainlines(数组) / archive_missing(可选 bool)"
            ))
        })?;

        let db = crate::global_state::get_sea_db().ok_or_else(|| {
            ToolError::execution_failed(
                "数据库未初始化，无法写入 market_mainlines（wiring 缺失：init 须调用 set_sea_db）"
                    .to_string(),
            )
        })?;

        let result = batch_upsert_mainlines(db.as_ref(), params)
            .await
            .map_err(|e| ToolError::execution_failed(format!("market_mainlines 写入失败: {e}")))?;

        let payload = serde_json::json!({
            "inserted": result.inserted,
            "updated": result.updated,
            "archived": result.archived,
        });
        let content = serde_json::to_string(&payload)
            .map_err(|e| ToolError::execution_failed(format!("序列化写入结果失败: {e}")))?;
        Ok(ToolResult::success(content))
    }
}

// ═══════════════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════════════
// 注意：本模块必须位于文件最末尾，否则触发 clippy::items_after_test_module。

#[cfg(test)]
mod tests {
    use super::*;

    /// 工具名是**跨文件契约**（seed 模板的 `ToolDef.name` 逐字匹配）。
    /// 这条断言是防「改名只改一侧」的锚点 —— 名字变了就必须同步模板 + 升 TEMPLATE_VERSION。
    #[test]
    fn tool_name_matches_seed_template_declaration() {
        let seed = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join("src/commands/stock_analysis_setup/seed_daily_market_events.rs"),
        )
        .expect("必须能读到 daily-market-events seed 文件（否则本断言的判据面消失）");
        assert!(
            seed.contains(&format!("name: \"{MARKET_MAINLINE_BATCH_UPSERT_TOOL}\".into()")),
            "seed_daily_market_events.rs 未声明工具 `{MARKET_MAINLINE_BATCH_UPSERT_TOOL}`。\
             模板与工具名必须逐字一致，否则运行时 ToolResolver 解析不到、调用被静默降级。"
        );
    }

    /// 负对照：证明上面那条断言**真的会因名字不符而红**（不是恒绿的空断言）。
    #[test]
    fn tool_name_assertion_discriminates() {
        let seed = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join("src/commands/stock_analysis_setup/seed_daily_market_events.rs"),
        )
        .unwrap();
        assert!(
            !seed.contains("name: \"market_mainline_batch_upsert_typo\".into()"),
            "负对照失败：一个不存在的名字竟然命中了 seed 文件"
        );
    }

    /// 参数解析必须收下 snake_case（提示词示例用的就是 snake_case）——
    /// 漏收会让 `archive_missing` 静默回落 false、「归档当日未提及主线」永不执行。
    ///
    /// ⚠️ 逐项对象**故意不带** `mainline_date`：提示词示例
    /// （`market-mainline/SKILL.md:116-134` / `market-synthesizer.md:27-46`）就是这样写的
    /// —— 日期只出现在**外层**。改前它是必填 ⇒ 这段 payload 报 `missing field`、
    /// **整批失败**（本用例首跑即红，抓出该缺陷）。不要把日期补进逐项对象来「修测试」。
    #[test]
    fn snake_case_payload_is_accepted() {
        let v = serde_json::json!({
            "mainline_date": "2026-09-20",
            "mainlines": [{
                "theme": "AI 算力",
                "theme_category": "科技",
                "narrative": "测试",
                "representative_symbols": ["300308"],
                "strength_score": 88,
                "persistence": "1w"
            }],
            "archive_missing": true
        });
        let p: BatchUpsertInput = serde_json::from_value(v).expect("snake_case 必须可解析");
        assert_eq!(p.mainline_date, "2026-09-20");
        assert!(p.archive_missing, "snake_case 的 archive_missing 必须被收下");
        assert_eq!(p.mainlines.len(), 1);
        assert!(
            p.mainlines[0].mainline_date.is_empty(),
            "逐项日期应留空并由外层提供（真实日期由 batch_upsert_mainlines 写入）"
        );
        assert_eq!(p.mainlines[0].theme_category, "科技", "theme_category 不得回落为「其他」");
        assert_eq!(p.mainlines[0].strength_score, 88.0, "strength_score 不得回落为 0");
        assert_eq!(p.mainlines[0].representative_symbols, vec!["300308".to_string()]);
    }

    /// camelCase 主名同样可解析（加 alias 不能把原契约弄坏）。
    #[test]
    fn camel_case_payload_still_accepted() {
        let v = serde_json::json!({
            "mainlineDate": "2026-09-20",
            "mainlines": [{
                "theme": "光模块",
                "themeCategory": "科技",
                "narrative": "测试",
                "representativeSymbols": ["002281"],
                "strengthScore": 70,
                "persistence": "1d"
            }],
            "archiveMissing": false
        });
        let p: BatchUpsertInput = serde_json::from_value(v).expect("camelCase 必须仍可解析");
        assert_eq!(p.mainline_date, "2026-09-20");
        assert!(!p.archive_missing);
        assert!(p.mainlines[0].mainline_date.is_empty());
        assert_eq!(p.mainlines[0].theme_category, "科技");
        assert_eq!(p.mainlines[0].strength_score, 70.0);
    }

    /// 负对照：外层日期缺失时**必须**报错（逐项可省 ≠ 整批可省）。
    #[test]
    fn missing_batch_date_is_an_error() {
        let v = serde_json::json!({
            "mainlines": [{ "theme": "x", "narrative": "y" }]
        });
        assert!(
            serde_json::from_value::<BatchUpsertInput>(v).is_err(),
            "外层 mainlineDate 缺失必须解析失败（否则会写出 date 为空的整批主线）"
        );
    }

    /// 缺 `mainline_date` ⇒ 必须报错，不得"成功"写到一个空日期。
    #[test]
    fn missing_required_field_is_an_error() {
        let v = serde_json::json!({ "mainlines": [] });
        assert!(
            serde_json::from_value::<BatchUpsertInput>(v).is_err(),
            "缺少 mainline_date 必须解析失败（否则会写出 date 为空的主线）"
        );
    }

    #[test]
    fn tool_metadata_is_consistent() {
        let t = MarketMainlineBatchUpsertTool;
        assert_eq!(t.name(), MARKET_MAINLINE_BATCH_UPSERT_TOOL);
        assert_eq!(t.domain(), ToolDomain::Finance, "护照域必须是 Finance");
        assert!(!t.is_read_only(), "会写库 ⇒ 非只读");
        let schema = t.input_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["properties"]["mainlines"]["type"], "array");
    }
}
