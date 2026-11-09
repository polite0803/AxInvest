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
//! 实现权威在 `axagent_analysis_engine::market_mainline`（implementor），但本 crate 是
//! consumer，按依赖铁律不得依赖 implementor（#240 E 批）⇒ 数据面改走插件命名命令
//! `stock_market_mainline_batch_upsert` → 宿主 `stock.analysis` 接缝（写 op）。SQL 与
//! DTO 归一都留在域包一侧。
//!
//! 宿主既有的 `src/commands/market_mainline.rs`（前端 `invoke` 与 chat 的
//! `execute_tauri_command`）仍是另一条独立通道，本批不动它。
//! - 本条通道：工作流 AgentNode / chat 工具调用
//! - `src/commands/market_mainline.rs`：前端 `invoke` 与 chat 的 `execute_tauri_command`
//!
//! # 参数契约
//!
//! `BatchUpsertInput` 的 serde 主名是 camelCase，但提示词示例（`market-mainline/SKILL.md`、
//! `market-synthesizer.md`）是 snake_case ⇒ 该 DTO 已对每个多词字段补 `alias`，
//! **两种写法都收**。`archive_missing` 尤甚：漏收会让「归档当日未提及主线」静默不执行。
//!
//! ⚠ 本层**不再**自己 serde 解析该 DTO（那需要 implementor 依赖）：形状校验归宿主
//! provider（`BatchUpsertInput` 的唯一解析点）。本层只做两件事：
//!
//! 1. 闸「外层日期缺失 / 空串」—— 那个字段缺失会写出 date 为空的一整批假主线，代价太高，
//!    不值得多跑一次命令帧才发现。
//! 2. 把外层 camelCase 别名收敛成清单 `required` 的 snake_case 主名（见
//!    [`normalize_outer_aliases`]）—— 插件闸是逐键 `contains_key`，不认 serde alias。

use crate::tools::plugin_bridge::call_plugin_command;
use crate::{Tool, ToolCategory, ToolContext, ToolDomain, ToolError, ToolResult};
use async_trait::async_trait;
use serde_json::Value;

/// 工具名 —— 必须与 `seed_daily_market_events.rs` 的 `ToolDef.name` **逐字相同**。
/// 改这里必须同步改模板（并递增 `TEMPLATE_VERSION` 触发重种子），否则模型调的还是旧名。
pub const MARKET_MAINLINE_BATCH_UPSERT_TOOL: &str = "market_mainline_batch_upsert";

/// 插件侧命令名 —— 必须与 `plugin-stock/src/main.rs` 的 `COMMAND_MAINLINE_BATCH_UPSERT`
/// 及插件清单（`requiredPermission: "workspace-write"`）**逐字相同**。
const COMMAND: &str = "stock_market_mainline_batch_upsert";

/// 插件 id（清单 `name`）。
const PLUGIN_ID: &str = "stock";

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

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // 闸：外层日期必须给且非空（snake/camel 两种写法都认）。
        //
        // 解析错**必须显式报错**，不能回落到默认值 —— 该 DTO 的字段多为
        // `#[serde(default)]`，一旦把 unknown/missing 当成"用默认值"，就会写出
        // strength_score=0、representative_symbols=[] 的假主线（"成功写入、数据全错"）。
        // 其余字段的形状校验归宿主 provider（DTO 的唯一解析点）。
        let date = input
            .get("mainline_date")
            .or_else(|| input.get("mainlineDate"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if date.trim().is_empty() {
            return Err(ToolError::invalid_input(
                "market_mainline_batch_upsert 缺少 mainline_date（YYYY-MM-DD）：\
                 外层日期缺失会写出 date 为空的整批主线",
            ));
        }

        let input = normalize_outer_aliases(input);
        let result = call_plugin_command(ctx, PLUGIN_ID, COMMAND, input.clone())
            .await
            .map_err(|e| ToolError::execution_failed(format!("market_mainlines 写入失败: {e}")))?;
        // 出参是本工具既有契约（`{inserted, updated, archived}`），逐字不变。
        let payload = serde_json::json!({
            "inserted": result.get("inserted").and_then(Value::as_u64).unwrap_or(0),
            "updated": result.get("updated").and_then(Value::as_u64).unwrap_or(0),
            "archived": result.get("archived").and_then(Value::as_u64).unwrap_or(0),
        });
        let content = serde_json::to_string(&payload)
            .map_err(|e| ToolError::execution_failed(format!("序列化写入结果失败: {e}")))?;
        Ok(ToolResult::success(content))
    }
}

/// 把外层 camelCase 别名收敛成插件清单 `required` 用的 snake_case 主名。
///
/// **为什么必须在工具层做**（2026-11-09 实证）：插件命令闸
/// （`plugins/src/manager.rs` 的 `check_command_input_shape`）是**逐键 `contains_key`**，
/// 只认真实 JSON Schema，不认 serde 的 `alias`。清单里
/// `stock_market_mainline_batch_upsert` 的 `required` 是 `["mainline_date", "mainlines"]`
/// ⇒ 只给 `mainlineDate` 的载荷会在**派发前**被拒（`缺少必填字段 mainline_date`），
/// 而本工具的 `input_schema` 明文承诺「也接受 camelCase 的 mainlineDate」。
///
/// 归一方向固定为 camel → snake：清单主名与全部提示词示例都是 snake，反向归一等于
/// 让清单跟着载荷改名。`mainlines` 逐项的字段**不动** —— 那是域 DTO 的事，
/// 插件闸只校验外层 `required`。
fn normalize_outer_aliases(input: Value) -> Value {
    const RENAMES: [(&str, &str); 3] = [
        ("mainlineDate", "mainline_date"),
        ("archiveMissing", "archive_missing"),
        ("sourceWorkflowExecutionId", "source_workflow_execution_id"),
    ];
    let Value::Object(map) = input else {
        return input;
    };
    let mut map = map;
    for (from, to) in RENAMES {
        // 两种写法都给时以 snake 为准（调用方显式写了主名就不该被别名顶掉）。
        if map.contains_key(to) {
            map.remove(from);
            continue;
        }
        if let Some(value) = map.remove(from) {
            map.insert(to.to_string(), value);
        }
    }
    Value::Object(map)
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
                .join("../../..")
                .join("src-tauri/src/commands/stock_analysis_setup/seed_daily_market_events.rs"),
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
                .join("../../..")
                .join("src-tauri/src/commands/stock_analysis_setup/seed_daily_market_events.rs"),
        )
        .unwrap();
        assert!(
            !seed.contains("name: \"market_mainline_batch_upsert_typo\".into()"),
            "负对照失败：一个不存在的名字竟然命中了 seed 文件"
        );
    }

    /// 缺外层日期 ⇒ 本层就报错，不得送进桥。
    ///
    /// 逐项日期可省（提示词示例就是这样：日期只出现在外层），但**外层**必须给 ——
    /// 缺它会写出 date 为空的整批主线。
    #[test]
    fn missing_batch_date_is_an_error() {
        let v = serde_json::json!({
            "mainlines": [{ "theme": "x", "narrative": "y" }]
        });
        assert!(v.get("mainline_date").is_none());
        assert!(v.get("mainlineDate").is_none());
        let t = MarketMainlineBatchUpsertTool;
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let err = rt.block_on(t.call(v, &ToolContext::new("."))).expect_err("缺外层日期必须报错");
        assert!(err.to_string().contains("缺少 mainline_date"), "实际: {err}");
        assert!(!err.to_string().contains("插件命令桥未注入"), "闸必须早于桥：{err}");
    }

    /// 空串日期同样要拒（`"   "` 会写成 `date='   '`）。
    #[test]
    fn blank_batch_date_is_an_error() {
        let t = MarketMainlineBatchUpsertTool;
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let err = rt
            .block_on(t.call(
                serde_json::json!({"mainline_date": "   ", "mainlines": []}),
                &ToolContext::new("."),
            ))
            .expect_err("空白日期必须报错");
        assert!(err.to_string().contains("缺少 mainline_date"), "实际: {err}");
    }

    /// camelCase 的日期主名也要认 —— 且**必须以 snake 主名进桥**。
    ///
    /// 插件命令闸（`plugins/src/manager.rs::check_command_input_shape`）是逐键
    /// `contains_key`，只认清单 `required` 里的 `mainline_date` / `mainlines`；
    /// 本工具的 schema 承诺收 camelCase ⇒ 归一必须在工具层完成，否则
    /// `{"mainlineDate": …}` 会在**派发前**被插件闸拒（工具按文档用不了）。
    /// 本测试模拟该闸的判据（不依赖插件 crate：tools 是 consumer，不得依赖 plugins）。
    #[test]
    fn camel_case_date_is_normalized_to_schema_required_keys() {
        #[derive(Debug)]
        struct GateStub;
        #[async_trait]
        impl axagent_harness::PluginCommandBridge for GateStub {
            async fn execute(
                &self,
                _plugin_id: &str,
                _command: &str,
                input: Value,
            ) -> Result<Value, String> {
                // 逐字复刻插件清单 `required: ["mainline_date", "mainlines"]` 的判据。
                for key in ["mainline_date", "mainlines"] {
                    if input.get(key).is_none() {
                        return Err(format!("缺少必填字段 `{key}`"));
                    }
                }
                Ok(serde_json::json!({"inserted": 0, "updated": 0, "archived": 0}))
            }
        }
        let t = MarketMainlineBatchUpsertTool;
        let ctx =
            ToolContext::new("mock").with_plugin_command_bridge(std::sync::Arc::new(GateStub));
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        rt.block_on(t.call(
            serde_json::json!({
                "mainlineDate": "2026-09-20",
                "mainlines": [{"theme": "AI 算力", "narrative": "测试"}],
                "archiveMissing": true
            }),
            &ctx,
        ))
        .expect("camelCase 载荷必须能穿过插件闸的 required 判据");
    }

    /// 归一不得顶掉显式写了主名的值（两种写法并存时 snake 优先）。
    #[test]
    fn explicit_snake_case_wins_over_camel_alias() {
        let v = normalize_outer_aliases(serde_json::json!({
            "mainline_date": "2026-09-20",
            "mainlineDate": "2026-08-01"
        }));
        assert_eq!(v["mainline_date"], "2026-09-20");
        assert!(v.get("mainlineDate").is_none(), "别名键必须被移除：{v}");
    }

    /// 非对象入参原样返回（归一不得把坏载荷变成另一种坏载荷）。
    #[test]
    fn non_object_input_is_returned_verbatim() {
        let v = normalize_outer_aliases(serde_json::json!(["x"]));
        assert_eq!(v, serde_json::json!(["x"]));
    }

    /// 桥未注入 ⇒ fail-closed（不回落直连 `analysis-engine`）。
    #[test]
    fn missing_bridge_fails_closed() {
        let t = MarketMainlineBatchUpsertTool;
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let err = rt
            .block_on(t.call(
                serde_json::json!({"mainline_date": "2026-09-20", "mainlines": []}),
                &ToolContext::new("."),
            ))
            .expect_err("桥缺失必须失败");
        assert!(err.to_string().contains("插件命令桥未注入"), "实际: {err}");
    }

    /// 入参**逐字**进桥（snake_case 原样保留：本层不做 snake→camel 改名），
    /// 出参 `{inserted, updated, archived}` 是本工具既有契约，逐字不变。
    #[test]
    fn passthrough_and_result_shape_are_verbatim() {
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
                assert_eq!(command, "stock_market_mainline_batch_upsert");
                assert_eq!(
                    input["mainline_date"], "2026-09-20",
                    "snake_case 必须原样进桥：{input}"
                );
                assert_eq!(input["archive_missing"], true, "archive_missing 不得被丢弃");
                Ok(self.0.clone())
            }
        }
        let t = MarketMainlineBatchUpsertTool;
        let ctx = ToolContext::new("mock").with_plugin_command_bridge(std::sync::Arc::new(Stub(
            serde_json::json!({"inserted": 2, "updated": 1, "archived": 3}),
        )));
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let out = rt
            .block_on(t.call(
                serde_json::json!({
                    "mainline_date": "2026-09-20",
                    "mainlines": [{"theme": "AI 算力", "narrative": "测试"}],
                    "archive_missing": true
                }),
                &ctx,
            ))
            .expect("桥桩应答应成功");
        let payload: Value = serde_json::from_str(&out.content).expect("出参必须是 JSON 文本");
        assert_eq!(payload["inserted"], 2);
        assert_eq!(payload["updated"], 1);
        assert_eq!(payload["archived"], 3);
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
