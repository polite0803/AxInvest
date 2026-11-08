// SPDX-License-Identifier: AGPL-3.0-only

//! OPC (一人公司) 业务工具
//!
//! 将 opc-dao 的数据操作暴露为 Agent Tool，
//! 使 AI Agent 能够直接管理发票、客户、项目等 OPC 业务数据。
//!
//! 所有工具使用 `ToolDomain::Automation` 分类，
//! 可通过域过滤仅暴露给 OPC 角色的 Agent。

use crate::{Tool, ToolCategory, ToolContext, ToolDomain, ToolError, ToolResult};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;

// ⚠ 已退役（缺陷 #184）：原先这里有
// `OpcNotification` + `static OPC_NOTIFY_TX: OnceLock<UnboundedSender<..>>`
// + `set_opc_notify_tx()` + `get_notify_tx()`，以及 `OpcSendNotificationTool`
// （`OpcSendNotification`）。退役理由：
//   ① `get_notify_tx()` 是 `OPC_NOTIFY_TX.get().expect("OPC notify tx not initialized")`
//      —— **同步 panic**，位于工具执行路径（`Tool::call`）内；
//   ② `set_opc_notify_tx` 全仓**零调用者** ⇒ 该 `OnceLock` 恒未初始化 ⇒ 一旦注册即必 panic；
//   ③ 更要害的是 `OpcNotification` 也**零接收端**：全仓没有任何 `Receiver` / 消费循环 /
//      通知 worker 读这条 channel。所以「补一个生产者」（在 wiring 层注入一次 tx）并不成立 ——
//      `UnboundedSender` 在无接收端时 `send` 恒 `Err`，补完只是把「必 panic」换成
//      「必失败」，正是任务 #115 对 `alert_tx` 判定的同一形态缺陷（「写进一条没人听的频道」）。
//      让它真正可用需要另建通知 worker + 渠道配置接线，属新功能，不在缺陷修复授权内。
// ⇒ 裁定：**退役该工具面**（与 `alert_tx` 同批收口口径）。panic 分支随实现体一并消失。
// ⚠ 各 OPC 域包 / 专家清单里原本还有 **31 处** `td("OpcSendNotification")` 之类的**声明**
//    （`commands/opc_workflows/seed_*.rs`、`commands/opc_setup/*`）—— 2026-11-07 任务 #208
//    已把它们全部清理，并同步递增 11 个逐文件 `TEMPLATE_VERSION`（让已安装库重导模板，
//    这是退役声明唯一生效路径）。配合 `config/agent_roles.yaml`、`config/opc/domain_packs/
//    sales_growth/manifest.yaml` 与 5 份 `agency_experts/opc/**` 专家 prompt 的同批去声明，
//    现在全仓（除文档与退役判据测试）已无该工具名的活声明。

fn get_db() -> Result<Arc<sea_orm::DatabaseConnection>, ToolError> {
    crate::global_state::get_sea_db()
        .ok_or_else(|| ToolError::execution_failed("OPC 数据库未初始化".to_string()))
}

/// 调 `plugin-opc` 的命名命令（#36 批 1：只读工具不再直连 `company-runtime`）。
///
/// 入参是**平铺的 camelCase 键**（`customerId` / `dateFrom` …），与 `opc.store` 查询 DTO
/// 的线格式同源；插件侧负责把它们包进接缝要求的 `request` 层。
/// 缺键不补 `null`：provider 侧 `Option` 与「缺省」同一语义，补 `null` 只是多传一个值。
async fn call_opc_command(ctx: &ToolContext, command: &str, input: Value) -> Result<Value, String> {
    let bridge = ctx
        .plugin_command_bridge
        .as_ref()
        .ok_or_else(|| format!("{command}: 插件命令桥未注入（fail-closed，不回落直连）"))?;
    bridge.execute("opc", command, input).await
}

/// 把工具入参里**调用方真给了**的键改名后装进一个平铺对象（缺键 / `null` 都不出现）。
fn filter_request(input: &Value, mapping: &[(&str, &str)]) -> Value {
    let mut request = serde_json::Map::new();
    for (from, to) in mapping {
        if let Some(v) = input.get(*from).filter(|v| !v.is_null()) {
            request.insert((*to).to_string(), v.clone());
        }
    }
    Value::Object(request)
}

/// 接缝出参 → 行数组（空表是空数组，不是错误）。
fn rows_of(value: Value) -> Vec<Value> {
    value.as_array().cloned().unwrap_or_default()
}

fn json_str(row: &Value, key: &str) -> String {
    row.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn json_f64(row: &Value, key: &str) -> f64 {
    row.get(key).and_then(Value::as_f64).unwrap_or_default()
}

fn json_u64(row: &Value, key: &str) -> u64 {
    row.get(key).and_then(Value::as_u64).unwrap_or_default()
}

/// epoch 秒 → 本地展示串；缺失 / 非法时间戳落空串（与原格式化路径一致）。
fn json_date(row: &Value, key: &str, fmt: &str) -> String {
    row.get(key).and_then(Value::as_i64).map(|ts| format_ts(ts, fmt)).unwrap_or_default()
}

fn format_ts(ts: i64, fmt: &str) -> String {
    chrono::DateTime::from_timestamp(ts, 0).map(|dt| dt.format(fmt).to_string()).unwrap_or_default()
}

// ═══════════════════════════════════════════════════════════════════
// 发票工具
// ═══════════════════════════════════════════════════════════════════

pub struct OpcListInvoicesTool;

#[async_trait]
impl Tool for OpcListInvoicesTool {
    fn name(&self) -> &str {
        "OpcListInvoices"
    }
    fn description(&self) -> &str {
        "查询一人公司(OPC)的发票列表。支持按状态、客户、日期范围过滤。返回发票摘要信息。"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["draft", "sent", "paid", "overdue", "cancelled", "refunded"],
                    "description": "按状态过滤（可选）"
                },
                "customer_id": { "type": "string", "description": "按客户ID过滤（可选）" },
                "date_from": { "type": "integer", "description": "起始时间戳（可选）" },
                "date_to": { "type": "integer", "description": "结束时间戳（可选）" },
                "limit": { "type": "integer", "description": "返回数量上限（默认10）" }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Integration
    }
    fn domain(&self) -> ToolDomain {
        ToolDomain::Automation
    }
    fn is_concurrency_safe(&self) -> bool {
        true
    }
    fn is_read_only(&self) -> bool {
        true
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // #36 批 1：取数走宿主 → 插件命令（`opc_list_invoices`），出参是
        // `InvoiceSummaryView` 行数组（camelCase，禁区 13）—— 本工具只负责排版。
        const COMMAND: &str = "opc_list_invoices";
        let request = filter_request(
            &input,
            &[
                ("status", "status"),
                ("customer_id", "customerId"),
                ("date_from", "dateFrom"),
                ("date_to", "dateTo"),
                ("limit", "limit"),
            ],
        );
        let invoices = call_opc_command(ctx, COMMAND, request)
            .await
            .map_err(|e| ToolError::execution_failed(format!("查询发票失败: {e}")))?;
        let rows = invoices.as_array().cloned().unwrap_or_default();
        if rows.is_empty() {
            return Ok(ToolResult::success("## 发票列表\n\n暂无发票记录。"));
        }
        let lines: Vec<String> = rows
            .iter()
            .map(|inv| {
                format!(
                    "- **{}** | {} | ¥{:.2} | {} | 客户: {}",
                    json_str(inv, "invoiceNumber"),
                    json_str(inv, "status"),
                    inv.get("total").and_then(Value::as_f64).unwrap_or_default(),
                    json_date(inv, "createdAt", "%Y-%m-%d"),
                    json_str(inv, "customerId"),
                )
            })
            .collect();
        Ok(ToolResult::success(format!(
            "## 发票列表 ({} 条)\n\n{}\n\n> 共 {} 条结果。使用 `OpcGetInvoice` 查看详情。",
            rows.len(),
            lines.join("\n"),
            rows.len(),
        )))
    }
}

pub struct OpcCreateInvoiceTool;

#[async_trait]
impl Tool for OpcCreateInvoiceTool {
    fn name(&self) -> &str {
        "OpcCreateInvoice"
    }
    fn description(&self) -> &str {
        "创建一张新发票。需要提供客户ID、行项目（描述、数量、单价、税率）、货币和可选备注。"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "customer_id": { "type": "string", "description": "客户ID" },
                "line_items": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "description": { "type": "string" },
                            "quantity": { "type": "number" },
                            "unit_price": { "type": "number" },
                            "tax_rate": { "type": "number", "description": "税率，如 0.13 表示13%" }
                        },
                        "required": ["description", "quantity", "unit_price"]
                    },
                    "description": "行项目列表。quantity×unit_price×(1+tax_rate) 自动计算 total"
                },
                "currency": { "type": "string", "default": "CNY", "description": "货币代码（默认CNY）" },
                "due_at": { "type": "integer", "description": "到期时间戳（可选）" },
                "notes": { "type": "string", "description": "备注（可选）" }
            },
            "required": ["customer_id", "line_items"]
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Integration
    }
    fn domain(&self) -> ToolDomain {
        ToolDomain::Automation
    }
    fn is_destructive(&self) -> bool {
        false
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // #36 批 3：写面走插件命名命令（`opc_create_invoice`）。行项目的缺省值（数量 1 /
        // 单价 0 / 税率 0）与行金额现算 `quantity × unitPrice × (1 + taxRate)` 全在宿主
        // provider —— 本进程不再链接 `axagent_company_runtime`，也不再抄一遍算式。
        const COMMAND: &str = "opc_create_invoice";
        let customer_id = input
            .get("customer_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("缺少必需参数: customer_id"))?;
        let line_items = input
            .get("line_items")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ToolError::invalid_input("缺少必需参数: line_items"))?
            .iter()
            .map(|item| {
                filter_request(
                    item,
                    &[
                        ("description", "description"),
                        ("quantity", "quantity"),
                        ("unit_price", "unitPrice"),
                        ("tax_rate", "taxRate"),
                    ],
                )
            })
            .collect::<Vec<_>>();
        let mut payload = filter_request(
            &input,
            &[("currency", "currency"), ("due_at", "dueAt"), ("notes", "notes")],
        );
        if let Some(map) = payload.as_object_mut() {
            map.insert("customerId".to_string(), json!(customer_id));
            map.insert("lineItems".to_string(), Value::Array(line_items));
        }

        let invoice = call_opc_command(ctx, COMMAND, payload)
            .await
            .map_err(|e| ToolError::execution_failed(format!("创建发票失败: {e}")))?;
        Ok(ToolResult::success(format!(
            "## 发票创建成功\n\n- **编号**: {}\n- **金额**: ¥{:.2}\n- **状态**: {}\n- **ID**: `{}`\n\n可使用 `OpcTransitionInvoice` 更改状态。",
            json_str(&invoice, "invoiceNumber"),
            json_f64(&invoice, "total"),
            json_str(&invoice, "status"),
            json_str(&invoice, "id"),
        )))
    }
}

pub struct OpcTransitionInvoiceTool;

#[async_trait]
impl Tool for OpcTransitionInvoiceTool {
    fn name(&self) -> &str {
        "OpcTransitionInvoice"
    }
    fn description(&self) -> &str {
        "变更发票状态。合法转换: draft→sent, draft→cancelled, sent→paid, sent→overdue, sent→cancelled, overdue→paid, overdue→cancelled, paid→refunded。"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "发票ID" },
                "status": {
                    "type": "string",
                    "enum": ["draft", "sent", "paid", "overdue", "cancelled", "refunded"],
                    "description": "目标状态"
                }
            },
            "required": ["id", "status"]
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Integration
    }
    fn domain(&self) -> ToolDomain {
        ToolDomain::Automation
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // #36 批 2：写面走插件命名命令（`opc_transition_invoice`），不再直连
        // `axagent_company_runtime::opc` 的 InvoiceService。命令面入参 `status` 由插件
        // 翻成契约 op 的 `target`；出参是外部标记枚举（`{"transitioned": …}` / `"not_found"`）。
        const COMMAND: &str = "opc_transition_invoice";
        let id = input
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("缺少必需参数: id"))?;
        let status = input
            .get("status")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("缺少必需参数: status"))?;

        let outcome = call_opc_command(ctx, COMMAND, json!({ "id": id, "status": status }))
            .await
            .map_err(|e| ToolError::execution_failed(format!("状态变更失败: {e}")))?;
        // 缺行是**业务读数**（不是故障）：明确告诉调用方未做任何变更。
        let Some(inv) = outcome.get("transitioned") else {
            return Ok(ToolResult::success(format!(
                "## 发票不存在\n\n`{id}` 不在发票表里，未做任何变更。"
            )));
        };
        Ok(ToolResult::success(format!(
            "## 发票状态已更新\n\n- **编号**: {}\n- **新状态**: **{}**\n- **金额**: ¥{:.2}",
            json_str(inv, "invoiceNumber"),
            json_str(inv, "status"),
            json_f64(inv, "total"),
        )))
    }
}

// ═══════════════════════════════════════════════════════════════════
// 客户工具
// ═══════════════════════════════════════════════════════════════════

pub struct OpcListCustomersTool;

#[async_trait]
impl Tool for OpcListCustomersTool {
    fn name(&self) -> &str {
        "OpcListCustomers"
    }
    fn description(&self) -> &str {
        "查询一人公司(OPC)的客户列表。支持按状态、名称/邮箱搜索过滤。"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["lead", "prospect", "active", "inactive", "churned"],
                    "description": "按客户状态过滤（可选）"
                },
                "search": { "type": "string", "description": "按名称/邮箱/公司模糊搜索（可选）" },
                "limit": { "type": "integer", "description": "返回数量上限（默认10）" }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Integration
    }
    fn domain(&self) -> ToolDomain {
        ToolDomain::Automation
    }
    fn is_concurrency_safe(&self) -> bool {
        true
    }
    fn is_read_only(&self) -> bool {
        true
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        const COMMAND: &str = "opc_list_customers";
        // `tags` 刻意不转发：迁移前该工具恒传 `tags: None`（`CustomerFilter` 里没有对应的
        // 工具入参），而域服务 `list_customers` 也没有 tags 分支 —— 转发只会造出「筛过了」
        // 的假象。要保持能力的批次再一起动工具 schema 与域服务。
        let request = filter_request(
            &input,
            &[
                ("status", "status"),
                ("customer_type", "customerType"),
                ("search", "search"),
                ("limit", "limit"),
            ],
        );
        let customers = call_opc_command(ctx, COMMAND, request)
            .await
            .map_err(|e| ToolError::execution_failed(format!("查询客户失败: {e}")))?;
        let rows = customers.as_array().cloned().unwrap_or_default();
        if rows.is_empty() {
            return Ok(ToolResult::success("## 客户列表\n\n暂无客户记录。"));
        }
        let lines: Vec<String> = rows
            .iter()
            .map(|c| {
                format!(
                    "- **{}** | {} | {} | 消费: ¥{:.2} | {}",
                    json_str(c, "name"),
                    json_str(c, "status"),
                    json_str(c, "email"),
                    json_f64(c, "totalRevenue"),
                    c.get("company").and_then(Value::as_str).unwrap_or(""),
                )
            })
            .collect();
        Ok(ToolResult::success(format!(
            "## 客户列表 ({} 条)\n\n{}\n\n> 共 {} 条结果。",
            rows.len(),
            lines.join("\n"),
            rows.len(),
        )))
    }
}

pub struct OpcCreateCustomerTool;

#[async_trait]
impl Tool for OpcCreateCustomerTool {
    fn name(&self) -> &str {
        "OpcCreateCustomer"
    }
    fn description(&self) -> &str {
        "创建新客户记录。需要提供姓名和邮箱，可选电话、公司、来源、标签和备注。"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "客户姓名" },
                "email": { "type": "string", "description": "客户邮箱" },
                "phone": { "type": "string", "description": "电话号码（可选）" },
                "company": { "type": "string", "description": "公司名称（可选）" },
                "source": {
                    "type": "string",
                    "enum": ["Referral", "Website", "SocialMedia", "Marketplace", "Direct"],
                    "description": "客户来源（可选）"
                },
                "notes": { "type": "string", "description": "备注（可选）" }
            },
            "required": ["name", "email"]
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Integration
    }
    fn domain(&self) -> ToolDomain {
        ToolDomain::Automation
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // #36 批 3：写面走插件命名命令（`opc_create_customer`）。`source` / `customer_type`
        // 的枚举映射（未知值落 `Other(原串)` / `Unknown`）在宿主 provider —— 本进程只做
        // 键名适配（工具入参沿用 snake_case，命令面线格式是 camelCase）。
        const COMMAND: &str = "opc_create_customer";
        let name = input
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("缺少必需参数: name"))?;
        let email = input
            .get("email")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("缺少必需参数: email"))?;

        let mut payload = filter_request(
            &input,
            &[
                ("phone", "phone"),
                ("company", "company"),
                ("customer_type", "customerType"),
                ("country", "country"),
                ("region", "region"),
                ("city", "city"),
                ("address", "address"),
                ("latitude", "latitude"),
                ("longitude", "longitude"),
                ("source", "source"),
                ("notes", "notes"),
            ],
        );
        if let Some(map) = payload.as_object_mut() {
            map.insert("name".to_string(), json!(name));
            map.insert("email".to_string(), json!(email));
        }

        let customer = call_opc_command(ctx, COMMAND, payload)
            .await
            .map_err(|e| ToolError::execution_failed(format!("创建客户失败: {e}")))?;
        Ok(ToolResult::success(format!(
            "## 客户创建成功\n\n- **姓名**: {}\n- **邮箱**: {}\n- **状态**: {}\n- **ID**: `{}`",
            json_str(&customer, "name"),
            json_str(&customer, "email"),
            json_str(&customer, "status"),
            json_str(&customer, "id"),
        )))
    }
}

// ═══════════════════════════════════════════════════════════════════
// 项目工具
// ═══════════════════════════════════════════════════════════════════

pub struct OpcListProjectsTool;

#[async_trait]
impl Tool for OpcListProjectsTool {
    fn name(&self) -> &str {
        "OpcListProjects"
    }
    fn description(&self) -> &str {
        "查询一人公司(OPC)的项目列表。支持按状态、客户过滤和关键词搜索。"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["planning", "active", "paused", "completed", "cancelled"],
                    "description": "按项目状态过滤（可选）"
                },
                "customer_id": { "type": "string", "description": "按客户ID过滤（可选）" },
                "search": { "type": "string", "description": "按标题/描述搜索（可选）" },
                "limit": { "type": "integer", "description": "返回数量上限（默认10）" }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Integration
    }
    fn domain(&self) -> ToolDomain {
        ToolDomain::Automation
    }
    fn is_concurrency_safe(&self) -> bool {
        true
    }
    fn is_read_only(&self) -> bool {
        true
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        const COMMAND: &str = "opc_list_projects";
        let request = filter_request(
            &input,
            &[
                ("status", "status"),
                ("customer_id", "customerId"),
                ("search", "search"),
                ("limit", "limit"),
            ],
        );
        let projects = call_opc_command(ctx, COMMAND, request)
            .await
            .map_err(|e| ToolError::execution_failed(format!("查询项目失败: {e}")))?;
        let rows = projects.as_array().cloned().unwrap_or_default();
        if rows.is_empty() {
            return Ok(ToolResult::success("## 项目列表\n\n暂无项目记录。"));
        }
        let lines: Vec<String> = rows
            .iter()
            .map(|p| {
                // 里程碑明细不过线：接缝只回两个计数（`ProjectSummaryView`）。
                let total = p.get("milestoneTotal").and_then(Value::as_u64).unwrap_or(0);
                let done = p.get("milestoneCompleted").and_then(Value::as_u64).unwrap_or(0);
                let milestone_info = if total == 0 {
                    String::new()
                } else {
                    format!(" (里程碑: {done}/{total})")
                };
                let budget_str = p
                    .get("budget")
                    .and_then(Value::as_f64)
                    .map(|b| format!(" | 预算: ¥{b:.2}"))
                    .unwrap_or_default();
                format!(
                    "- **{}** | {}{}{}",
                    json_str(p, "title"),
                    json_str(p, "status"),
                    milestone_info,
                    budget_str,
                )
            })
            .collect();
        Ok(ToolResult::success(format!("## 项目列表 ({} 条)\n\n{}", rows.len(), lines.join("\n"),)))
    }
}

pub struct OpcCreateProjectTool;

#[async_trait]
impl Tool for OpcCreateProjectTool {
    fn name(&self) -> &str {
        "OpcCreateProject"
    }
    fn description(&self) -> &str {
        "创建新项目。需要提供项目标题，可选关联客户、描述、预算和截止日期。"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "项目标题" },
                "description": { "type": "string", "description": "项目描述（可选）" },
                "customer_id": { "type": "string", "description": "关联客户ID（可选）" },
                "budget": { "type": "number", "description": "项目预算（可选）" },
                "currency": { "type": "string", "default": "CNY", "description": "货币代码（默认CNY）" },
                "deadline": { "type": "integer", "description": "截止时间戳（可选）" },
                "notes": { "type": "string", "description": "备注（可选）" }
            },
            "required": ["title"]
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Integration
    }
    fn domain(&self) -> ToolDomain {
        ToolDomain::Automation
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // #36 批 3：写面走插件命名命令（`opc_create_project`）。缺省补值（描述空串 /
        // 货币 CNY / 备注空串）在宿主 provider —— 本进程只做键名适配。
        const COMMAND: &str = "opc_create_project";
        let title = input
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("缺少必需参数: title"))?;
        let mut payload = filter_request(
            &input,
            &[
                ("customer_id", "customerId"),
                ("description", "description"),
                ("budget", "budget"),
                ("currency", "currency"),
                ("deadline", "deadline"),
                ("notes", "notes"),
            ],
        );
        if let Some(map) = payload.as_object_mut() {
            map.insert("title".to_string(), json!(title));
        }

        let project = call_opc_command(ctx, COMMAND, payload)
            .await
            .map_err(|e| ToolError::execution_failed(format!("创建项目失败: {e}")))?;
        Ok(ToolResult::success(format!(
            "## 项目创建成功\n\n- **标题**: {}\n- **状态**: {}\n- **ID**: `{}`\n\n可使用 `OpcAddMilestone` 添加里程碑。",
            json_str(&project, "title"),
            json_str(&project, "status"),
            json_str(&project, "id"),
        )))
    }
}

pub struct OpcAddMilestoneTool;

#[async_trait]
impl Tool for OpcAddMilestoneTool {
    fn name(&self) -> &str {
        "OpcAddMilestone"
    }
    fn description(&self) -> &str {
        "为项目添加里程碑。需要项目ID、里程碑标题和描述。"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "project_id": { "type": "string", "description": "项目ID" },
                "title": { "type": "string", "description": "里程碑标题" },
                "description": { "type": "string", "description": "里程碑描述（可选）" },
                "due_at": { "type": "integer", "description": "到期时间戳（可选）" }
            },
            "required": ["project_id", "title"]
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Integration
    }
    fn domain(&self) -> ToolDomain {
        ToolDomain::Automation
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // #36 批 3：写面走插件命名命令（`opc_add_milestone`）。里程碑 id 生成与初始状态
        // （`Pending`）在宿主 provider —— 本进程只传用户给的四个字段。
        const COMMAND: &str = "opc_add_milestone";
        let project_id = input
            .get("project_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("缺少必需参数: project_id"))?;
        let title = input
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("缺少必需参数: title"))?;
        let mut payload =
            filter_request(&input, &[("description", "description"), ("due_at", "dueAt")]);
        if let Some(map) = payload.as_object_mut() {
            map.insert("projectId".to_string(), json!(project_id));
            map.insert("title".to_string(), json!(title));
        }

        let project = call_opc_command(ctx, COMMAND, payload)
            .await
            .map_err(|e| ToolError::execution_failed(format!("添加里程碑失败: {e}")))?;
        Ok(ToolResult::success(format!(
            "## 里程碑已添加\n\n- **项目**: {}\n- **里程碑数**: {}\n\n可使用 `OpcCompleteMilestone` 标记完成。",
            json_str(&project, "title"),
            json_u64(&project, "milestoneTotal"),
        )))
    }
}

// ═══════════════════════════════════════════════════════════════════
// 仪表盘工具
// ═══════════════════════════════════════════════════════════════════

pub struct OpcGetDashboardTool;

#[async_trait]
impl Tool for OpcGetDashboardTool {
    fn name(&self) -> &str {
        "OpcGetDashboard"
    }
    fn description(&self) -> &str {
        "获取一人公司(OPC)的运营概览。返回总收入、发票数、活跃项目、客户总数等关键指标。"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Integration
    }
    fn domain(&self) -> ToolDomain {
        ToolDomain::Automation
    }
    fn is_concurrency_safe(&self) -> bool {
        true
    }
    fn is_read_only(&self) -> bool {
        true
    }

    async fn call(&self, _input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // 聚合口径住在插件进程（`opc_get_dashboard` 内部串行调三个 list op，
        // 判据同迁移前：Paid 求和 / Draft|Sent|Overdue 计数 / Active|Planning / Active）——
        // #36 的判据是「业务逻辑随域迁出」，宿主工具只排版。
        let summary = call_opc_command(ctx, "opc_get_dashboard", json!({}))
            .await
            .map_err(|e| ToolError::execution_failed(format!("查询运营概览失败: {e}")))?;
        let total_revenue = json_f64(&summary, "totalRevenue");
        let pending_invoices = json_u64(&summary, "pendingInvoices");
        let active_projects = json_u64(&summary, "activeProjects");
        let active_customers = json_u64(&summary, "activeCustomers");
        let total_customers = json_u64(&summary, "totalCustomers");
        let total_projects = json_u64(&summary, "totalProjects");

        Ok(ToolResult::success(format!(
            r#"## 📊 OPC 运营概览

| 指标 | 数值 |
|------|------|
| **总收入 (已收款)** | ¥{total_revenue:.2} |
| **待处理发票** | {pending_invoices} 张 |
| **活跃项目** | {active_projects} 个 |
| **活跃客户** | {active_customers} 位 |
| **客户总数** | {total_customers} 位 |
| **项目总数** | {total_projects} 个 |

> 使用 `OpcListInvoices` / `OpcListCustomers` / `OpcListProjects` 查看详情。"#,
            total_revenue = total_revenue,
            pending_invoices = pending_invoices,
            active_projects = active_projects,
            active_customers = active_customers,
            total_customers = total_customers,
            total_projects = total_projects,
        )))
    }
}

// ═══════════════════════════════════════════════════════════════════
// 站点/博客工具
// ═══════════════════════════════════════════════════════════════════

pub struct OpcListLandingPagesTool;

#[async_trait]
impl Tool for OpcListLandingPagesTool {
    fn name(&self) -> &str {
        "OpcListLandingPages"
    }
    fn description(&self) -> &str {
        "列出所有落地页。返回标题、slug、发布状态等摘要信息。"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{}})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Integration
    }
    fn domain(&self) -> ToolDomain {
        ToolDomain::Automation
    }
    fn is_concurrency_safe(&self) -> bool {
        true
    }
    fn is_read_only(&self) -> bool {
        true
    }

    async fn call(&self, _input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let pages = call_opc_command(ctx, "opc_list_landing_pages", json!({}))
            .await
            .map_err(|e| ToolError::execution_failed(format!("查询落地页失败: {e}")))?;
        let rows = rows_of(pages);
        if rows.is_empty() {
            return Ok(ToolResult::success("## 落地页\n\n暂无落地页。"));
        }
        let lines: Vec<String> = rows
            .iter()
            .map(|p| {
                format!(
                    "- **{}** (`{}`) {} | 创建: {}",
                    json_str(p, "title"),
                    json_str(p, "slug"),
                    if p.get("published").and_then(Value::as_bool).unwrap_or(false) {
                        "✅ 已发布"
                    } else {
                        "📝 草稿"
                    },
                    json_date(p, "createdAt", "%Y-%m-%d"),
                )
            })
            .collect();
        Ok(ToolResult::success(format!("## 落地页 ({} 个)\n\n{}", rows.len(), lines.join("\n"))))
    }
}

pub struct OpcListBlogPostsTool;

#[async_trait]
impl Tool for OpcListBlogPostsTool {
    fn name(&self) -> &str {
        "OpcListBlogPosts"
    }
    fn description(&self) -> &str {
        "列出所有博客文章。返回标题、slug、标签、发布状态。"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{}})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Integration
    }
    fn domain(&self) -> ToolDomain {
        ToolDomain::Automation
    }
    fn is_concurrency_safe(&self) -> bool {
        true
    }
    fn is_read_only(&self) -> bool {
        true
    }

    async fn call(&self, _input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // #36 blog 刀：取数走插件命名命令（plugin-opc 的 `opc_list_blog_posts`，整表
        // 分页取尽），不再直连 `axagent_company_runtime::opc` 的 SiteService。
        // 出参是 opc.store 接缝视图行数组（键名 camelCase，禁区 13）。
        const COMMAND: &str = "opc_list_blog_posts";
        let bridge = ctx
            .plugin_command_bridge
            .as_ref()
            .ok_or_else(|| ToolError::execution_failed(format!("{COMMAND}: 插件命令桥未注入")))?;
        let frame = bridge
            .execute("opc", COMMAND, serde_json::json!({}))
            .await
            .map_err(|e| ToolError::execution_failed(format!("查询博客失败: {e}")))?;
        let posts = frame.as_array().cloned().unwrap_or_default();
        if posts.is_empty() {
            return Ok(ToolResult::success("## 博客文章\n\n暂无文章。"));
        }
        let lines: Vec<String> = posts
            .iter()
            .map(|p| {
                let tags: Vec<String> = p
                    .get("tags")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                let tags = tags.join(", ");
                format!(
                    "- **{}** (`{}`) {} | 阅读: {} | 标签: {}",
                    p.get("title").and_then(Value::as_str).unwrap_or_default(),
                    p.get("slug").and_then(Value::as_str).unwrap_or_default(),
                    if p.get("published").and_then(Value::as_bool).unwrap_or(false) {
                        "✅ 已发布"
                    } else {
                        "📝 草稿"
                    },
                    p.get("viewCount").and_then(Value::as_i64).unwrap_or(0),
                    tags,
                )
            })
            .collect();
        Ok(ToolResult::success(format!("## 博客文章 ({} 篇)\n\n{}", posts.len(), lines.join("\n"))))
    }
}

pub struct OpcListContactsTool;

#[async_trait]
impl Tool for OpcListContactsTool {
    fn name(&self) -> &str {
        "OpcListContacts"
    }
    fn description(&self) -> &str {
        "列出所有联系表单提交。返回姓名、邮箱、来源、读取状态。"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{}})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Integration
    }
    fn domain(&self) -> ToolDomain {
        ToolDomain::Automation
    }
    fn is_concurrency_safe(&self) -> bool {
        true
    }
    fn is_read_only(&self) -> bool {
        true
    }

    async fn call(&self, _input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let contacts = call_opc_command(ctx, "opc_list_contacts", json!({}))
            .await
            .map_err(|e| ToolError::execution_failed(format!("查询联系表单失败: {e}")))?;
        let rows = rows_of(contacts);
        if rows.is_empty() {
            return Ok(ToolResult::success("## 联系表单\n\n暂无提交。"));
        }
        let lines: Vec<String> = rows
            .iter()
            .map(|c| {
                format!(
                    "- **{}** <{}> | {} | {}",
                    json_str(c, "name"),
                    json_str(c, "email"),
                    json_str(c, "source"),
                    if c.get("read").and_then(Value::as_bool).unwrap_or(false) {
                        "✅ 已读"
                    } else {
                        "🆕 未读"
                    },
                )
            })
            .collect();
        Ok(ToolResult::success(format!("## 联系表单 ({} 条)\n\n{}", rows.len(), lines.join("\n"))))
    }
}

pub struct OpcCreateLandingPageTool;

#[async_trait]
impl Tool for OpcCreateLandingPageTool {
    fn name(&self) -> &str {
        "OpcCreateLandingPage"
    }
    fn description(&self) -> &str {
        "创建新的落地页。需要提供标题、slug 和正文内容，可选描述。创建后为草稿状态。"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "落地页标题" },
                "slug": { "type": "string", "description": "URL slug（小写，空格自动转横线）" },
                "description": { "type": "string", "description": "页面描述（可选）" },
                "content": { "type": "string", "description": "页面正文内容（支持 Markdown）" }
            },
            "required": ["title", "slug", "content"]
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Integration
    }
    fn domain(&self) -> ToolDomain {
        ToolDomain::Automation
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // #36 批 3：写面走插件命名命令（`opc_create_landing_page`）。
        const COMMAND: &str = "opc_create_landing_page";
        let title = input
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("缺少必需参数: title"))?;
        let slug = input
            .get("slug")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("缺少必需参数: slug"))?;
        let content = input
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("缺少必需参数: content"))?;
        let mut payload = filter_request(&input, &[("description", "description")]);
        if let Some(map) = payload.as_object_mut() {
            map.insert("title".to_string(), json!(title));
            map.insert("slug".to_string(), json!(slug));
            map.insert("content".to_string(), json!(content));
        }

        let page = call_opc_command(ctx, COMMAND, payload)
            .await
            .map_err(|e| ToolError::execution_failed(format!("创建落地页失败: {e}")))?;
        Ok(ToolResult::success(format!(
            "## 落地页创建成功\n\n- **标题**: {}\n- **Slug**: `{}`\n- **状态**: 📝 草稿\n- **ID**: `{}`",
            json_str(&page, "title"),
            json_str(&page, "slug"),
            json_str(&page, "id"),
        )))
    }
}

pub struct OpcCreateBlogPostTool;

#[async_trait]
impl Tool for OpcCreateBlogPostTool {
    fn name(&self) -> &str {
        "OpcCreateBlogPost"
    }
    fn description(&self) -> &str {
        "创建新的博客文章。需要提供标题、slug 和正文内容，可选摘要和标签。创建后为草稿状态。"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "文章标题" },
                "slug": { "type": "string", "description": "URL slug（小写，空格自动转横线）" },
                "excerpt": { "type": "string", "description": "文章摘要（可选）" },
                "content": { "type": "string", "description": "文章正文内容（支持 Markdown）" },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "标签列表（可选）"
                }
            },
            "required": ["title", "slug", "content"]
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Integration
    }
    fn domain(&self) -> ToolDomain {
        ToolDomain::Automation
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // #36 blog 刀：写面走插件命名命令（plugin-opc 的 `opc_create_blog_post`，
        // 平铺五字段），不再直连 `axagent_company_runtime::opc` 的 SiteService。
        // 回帧是 opc.store 契约视图 BlogPostView（键名 camelCase，禁区 13）。
        const COMMAND: &str = "opc_create_blog_post";
        let bridge = ctx
            .plugin_command_bridge
            .as_ref()
            .ok_or_else(|| ToolError::execution_failed(format!("{COMMAND}: 插件命令桥未注入")))?;

        let title = input
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("缺少必需参数: title"))?;
        let slug = input
            .get("slug")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("缺少必需参数: slug"))?;
        let content = input
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("缺少必需参数: content"))?;

        let tags: Vec<String> = input
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        let frame = bridge
            .execute(
                "opc",
                COMMAND,
                serde_json::json!({
                    "title": title,
                    "slug": slug,
                    "excerpt": input.get("excerpt").and_then(|v| v.as_str()).unwrap_or(""),
                    "content": content,
                    "tags": tags,
                }),
            )
            .await
            .map_err(|e| ToolError::execution_failed(format!("创建博客文章失败: {e}")))?;

        let p_tags: Vec<String> = frame
            .get("tags")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        Ok(ToolResult::success(format!(
            "## 博客文章创建成功\n\n- **标题**: {}\n- **Slug**: `{}`\n- **标签**: {}\n- **状态**: 📝 草稿\n- **ID**: `{}`",
            frame.get("title").and_then(Value::as_str).unwrap_or_default(),
            frame.get("slug").and_then(Value::as_str).unwrap_or_default(),
            if p_tags.is_empty() {
                "无".to_string()
            } else {
                p_tags.join(", ")
            },
            frame.get("id").and_then(Value::as_str).unwrap_or_default(),
        )))
    }
}

// ═══════════════════════════════════════════════════════════════════
// 分析仪表盘工具
// ═══════════════════════════════════════════════════════════════════

pub struct OpcRecordKpiTool;

#[async_trait]
impl Tool for OpcRecordKpiTool {
    fn name(&self) -> &str {
        "OpcRecordKpi"
    }
    fn description(&self) -> &str {
        "记录一个 KPI 指标。**当前不可用**：KPI 必须归属某个域包，而本工具的运行上下文\
         不携带域包标识，调用必然失败（由 LLM 自报域包会写入错归属的记录）。\
         创作类 KPI 由域包工作流的 post_exec 钩子自动落库，无需手工记录。"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"name":{"type":"string"},"value":{"type":"number"},"unit":{"type":"string"},"period":{"type":"string"}},"required":["name","value","unit","period"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Integration
    }
    fn domain(&self) -> ToolDomain {
        ToolDomain::Automation
    }

    async fn call(&self, _input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // ⛔ 本工具**故意拒绝执行**（v230 起 KPI 写入必须带域包归属，而本工具拿不到它）。
        //
        // 这不是「还没实现」，是一条有意的取舍，三种更省事的写法都已被否决：
        // - **让 LLM 在入参里报一个 `domain_pack_id`**：`opc_kpi_records` 是跨域包共用的
        //   一张表，LLM 自报的域包要么是幻觉（落到不存在的域包桶）、要么是猜的
        //   （落到别人的域包桶）—— 都是「看起来成功的错数据」，比拒绝写入更糟。
        // - **空串 / 默认域包兜底**：等于把「调用方忘传」变成「静默写一行查不出来的
        //   记录」，与 v230 摘掉列 DEFAULT 的取舍直接矛盾。
        // - **从 `ToolContext.extra` 取**：`extra` 目前只有搜索配置与 `vault_kb_id`
        //   （`crates/tools/src/registry.rs:529-530` 的 `tool_extra`，
        //   `commands/agent/mod.rs` 是唯一注入点），**没有任何域包键** —— 现在从这里取
        //   只会恒为 `None`，等于给一个不存在的通道留档位。
        //
        // 域包归属的真实来源是**运行上下文**（工具在 OPC 域包工作流里被调用时，
        // 上下文应携带域包）。该接线结论由并行的「工具接线」勘察给出；在它落地前，
        // 本工具保持「响亮拒绝」而不是写无归属的行。
        //
        // 正常路径不依赖本工具：创作类 KPI 由域包工作流的 post_exec 钩子
        // （`content-media-kpi-persist`）在终态自动落库（见
        // `src/commands/opc_workflow_kpi_hook.rs`）。
        Err(ToolError::execution_failed(
            "OpcRecordKpi 不可用：KPI 记录必须归属某个域包，而本工具的运行上下文不携带\
             域包标识；由 LLM 自报域包会写入错归属的记录，故拒绝执行。\
             创作类 KPI 由域包工作流的 post_exec 钩子自动落库，无需手工记录。",
        ))
    }
}

pub struct OpcListKpisTool;

#[async_trait]
impl Tool for OpcListKpisTool {
    fn name(&self) -> &str {
        "OpcListKpis"
    }
    fn description(&self) -> &str {
        "查询 KPI 记录。可按周期过滤。"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"period":{"type":"string"},"limit":{"type":"integer"}}})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Integration
    }
    fn domain(&self) -> ToolDomain {
        ToolDomain::Automation
    }
    fn is_concurrency_safe(&self) -> bool {
        true
    }
    fn is_read_only(&self) -> bool {
        true
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let request = filter_request(&input, &[("period", "period"), ("limit", "limit")]);
        // 跨域包语义由接缝的 `list_kpis` op 承载（provider 走 `list_kpis_all`，
        // 判据同迁移前：调用方不携带域包标识，猜一个域包 id 是对别的域包的读）。
        // 每行都带 `domainPackId`，`''` 表示「未标注」而非某个域包。
        let kpis = call_opc_command(ctx, "opc_list_kpis", request)
            .await
            .map_err(|e| ToolError::execution_failed(format!("查询 KPI 失败: {e}")))?;
        let rows = rows_of(kpis);
        if rows.is_empty() {
            return Ok(ToolResult::success("## KPI 记录\n\n暂无记录。"));
        }
        Ok(ToolResult::success(format!(
            "## KPI 记录（跨域包，每条已标注所属域包）\n\n{}",
            rows.iter()
                .map(|k| {
                    let pack = json_str(k, "domainPackId");
                    let ind = if pack.is_empty() {
                        "(未标注)".to_string()
                    } else {
                        pack
                    };
                    format!(
                        "- [{ind}] {}: {} {} ({})",
                        json_str(k, "name"),
                        json_f64(k, "value"),
                        json_str(k, "unit"),
                        json_str(k, "period"),
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        )))
    }
}

// ═══════════════════════════════════════════════════════════════════
// 知识源工具
// ═══════════════════════════════════════════════════════════════════

pub struct OpcSearchWikiTool;

#[async_trait]
impl Tool for OpcSearchWikiTool {
    fn name(&self) -> &str {
        "OpcSearchWiki"
    }
    fn description(&self) -> &str {
        "在 OPC 业务 Wiki 中搜索知识文档（发票规范、客户管理、税法、模板等）。返回最匹配的文档内容摘要。"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"query":{"type":"string"},"top_k":{"type":"integer","default":3}},"required":["query"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Knowledge
    }
    fn domain(&self) -> ToolDomain {
        ToolDomain::Automation
    }
    fn is_read_only(&self) -> bool {
        true
    }
    fn is_concurrency_safe(&self) -> bool {
        true
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let query = input
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("缺少 query"))?
            .to_string();
        let top_k = input.get("top_k").and_then(|v| v.as_u64()).unwrap_or(3) as usize;

        let db = get_db()?;

        // 查找 OPC Wiki ID（走 dao repo，保持 tools 不依赖 entities）
        let wikis = axagent_dao::repo::wiki::list_wikis(&db)
            .await
            .map_err(|e| ToolError::execution_failed(format!("查询 Wiki 失败: {e}")))?;
        let wiki_id = match wikis.iter().find(|w| w.name == "OPC 业务知识库") {
            Some(w) => w.id.clone(),
            None => {
                return Ok(ToolResult::success(
                    "## OPC Wiki\n\nOPC 业务知识库尚未创建。系统将在首次启动后自动创建。",
                ));
            },
        };

        // 在 notes 表中按标题和内容模糊搜索（dao repo 已实现排序 + limit）
        let notes = axagent_dao::repo::note::search_notes(&db, &wiki_id, &query, top_k)
            .await
            .map_err(|e| ToolError::execution_failed(format!("搜索失败: {e}")))?;

        if notes.is_empty() {
            return Ok(ToolResult::success(format!(
                "## OPC 知识库搜索\n\n未找到包含「{query}」的文档。"
            )));
        }

        let lines: Vec<String> = notes
            .iter()
            .map(|n| {
                let excerpt = n
                    .content
                    .lines()
                    .filter(|l| l.contains(&query) || l.starts_with("# "))
                    .take(3)
                    .collect::<Vec<_>>()
                    .join("\n  ");
                format!("- **{}**\n  {}", n.title, excerpt)
            })
            .collect();

        Ok(ToolResult::success(format!(
            "## OPC 知识库搜索「{}」\n\n找到 {} 条结果\n\n{}",
            query,
            notes.len(),
            lines.join("\n\n")
        )))
    }
}

// ═══════════════════════════════════════════════════════════════════
// 投顾经营工具
// ═══════════════════════════════════════════════════════════════════

pub struct OpcGetFinancialReportTool;

#[async_trait]
impl Tool for OpcGetFinancialReportTool {
    fn name(&self) -> &str {
        "OpcGetFinancialReport"
    }
    fn description(&self) -> &str {
        "生成 OPC 财务报表。按期间（如 '2026-07' 或 '2026-Q2'）统计收入、利润、待收款、可投资金额。给出投资建议。"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"period":{"type":"string","description":"期间，如 '2026-07'（月度）或 '2026-Q2'（季度）"}},"required":["period"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Integration
    }
    fn domain(&self) -> ToolDomain {
        ToolDomain::Automation
    }
    fn is_read_only(&self) -> bool {
        true
    }
    fn is_concurrency_safe(&self) -> bool {
        true
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // #36 批 3：报表与投资建议由宿主 `get_financial_report` op 成对取数并拼成一份视图
        //（分两个 op 只会多一次帧往返）—— 本进程只排版，不再链接 `DefaultFinanceService`。
        const COMMAND: &str = "opc_get_financial_report";
        let period = input
            .get("period")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("缺少 period"))?;

        let report = call_opc_command(ctx, COMMAND, json!({ "period": period }))
            .await
            .map_err(|e| ToolError::execution_failed(format!("生成财务报表失败: {e}")))?;
        Ok(ToolResult::success(format!(
            r#"## 📊 OPC 财务报表: {}

| 指标 | 数值 |
|------|------|
| **总收入** | ¥{:.2} |
| **待收款** | ¥{:.2} |
| **退款** | ¥{:.2} |
| **净利润** | **¥{:.2}** |
| **可投资金额** | ¥{:.2} |
| **税前收入(估)** | ¥{:.2} |
| **活跃项目** | {} |
| **新增客户** | {} |
| **客户流失** | {} |

## 💡 投资建议

- **风险等级**: {}
- **建议金额**: ¥{:.2}
- **建议**: {}"#,
            json_str(&report, "period"),
            json_f64(&report, "totalRevenue"),
            json_f64(&report, "pendingRevenue"),
            json_f64(&report, "refundedAmount"),
            json_f64(&report, "netProfit"),
            json_f64(&report, "investableAmount"),
            json_f64(&report, "pretaxRevenue"),
            json_u64(&report, "activeProjects"),
            json_u64(&report, "newCustomers"),
            json_u64(&report, "churnedCustomers"),
            json_str(&report, "riskLevel"),
            json_f64(&report, "recommendedAmount"),
            json_str(&report, "advice"),
        )))
    }
}

// ═══════════════════════════════════════════════════════════════════
// 内容资产工具
// ═══════════════════════════════════════════════════════════════════

pub struct OpcCreateContentAssetTool;

#[async_trait]
impl Tool for OpcCreateContentAssetTool {
    fn name(&self) -> &str {
        "OpcCreateContentAsset"
    }
    fn description(&self) -> &str {
        "创建新的内容资产（文章、视频、图片等）。需要提供标题、内容类型和正文内容。创建后为草稿状态。"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "内容标题" },
                "content_type": {
                    "type": "string",
                    "enum": ["article", "video", "image"],
                    "description": "内容类型"
                },
                "body": { "type": "string", "description": "正文内容（支持 Markdown）" },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "标签列表（可选）"
                }
            },
            "required": ["title", "content_type", "body"]
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Integration
    }
    fn domain(&self) -> ToolDomain {
        ToolDomain::Automation
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // #36 批 3：写面走插件命名命令（`opc_create_content_asset`）。工具入参沿用
        // `content_type`，命令面线格式是 `contentType`（禁区 13）。
        const COMMAND: &str = "opc_create_content_asset";
        let title = input
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("缺少必需参数: title"))?;
        let content_type = input
            .get("content_type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("缺少必需参数: content_type"))?;
        let body = input
            .get("body")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("缺少必需参数: body"))?;
        let mut payload = filter_request(&input, &[("tags", "tags")]);
        if let Some(map) = payload.as_object_mut() {
            map.insert("title".to_string(), json!(title));
            map.insert("contentType".to_string(), json!(content_type));
            map.insert("body".to_string(), json!(body));
        }

        let asset = call_opc_command(ctx, COMMAND, payload)
            .await
            .map_err(|e| ToolError::execution_failed(format!("创建内容资产失败: {e}")))?;
        Ok(ToolResult::success(format!(
            "## 内容资产创建成功\n\n- **标题**: {}\n- **类型**: {}\n- **状态**: 📝 草稿\n- **ID**: `{}`",
            json_str(&asset, "title"),
            json_str(&asset, "contentType"),
            json_str(&asset, "id"),
        )))
    }
}

pub struct OpcListContentAssetsTool;

#[async_trait]
impl Tool for OpcListContentAssetsTool {
    fn name(&self) -> &str {
        "OpcListContentAssets"
    }
    fn description(&self) -> &str {
        "列出所有内容资产。返回标题、类型、标签、状态等摘要信息。"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{}})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Integration
    }
    fn domain(&self) -> ToolDomain {
        ToolDomain::Automation
    }
    fn is_concurrency_safe(&self) -> bool {
        true
    }
    fn is_read_only(&self) -> bool {
        true
    }

    async fn call(&self, _input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let assets = call_opc_command(ctx, "opc_list_content_assets", json!({}))
            .await
            .map_err(|e| ToolError::execution_failed(format!("查询内容资产失败: {e}")))?;
        let rows = rows_of(assets);
        if rows.is_empty() {
            return Ok(ToolResult::success("## 内容资产\n\n暂无内容资产。"));
        }
        let lines: Vec<String> = rows
            .iter()
            .map(|a| {
                let tags = a
                    .get("tags")
                    .and_then(Value::as_array)
                    .map(|arr| arr.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
                    .unwrap_or_default();
                format!(
                    "- **{}** | {} | {} | 标签: {}",
                    json_str(a, "title"),
                    json_str(a, "contentType"),
                    if json_str(a, "status") == "published" {
                        "✅ 已发布"
                    } else {
                        "📝 草稿"
                    },
                    tags,
                )
            })
            .collect();
        Ok(ToolResult::success(format!("## 内容资产 ({} 个)\n\n{}", rows.len(), lines.join("\n"))))
    }
}

pub struct OpcUpdateContentAssetTool;

#[async_trait]
impl Tool for OpcUpdateContentAssetTool {
    fn name(&self) -> &str {
        "OpcUpdateContentAsset"
    }
    fn description(&self) -> &str {
        "更新内容资产。可选择性更新标题、类型、正文、标签和状态。"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "内容资产ID" },
                "title": { "type": "string", "description": "新标题（可选）" },
                "content_type": {
                    "type": "string",
                    "enum": ["article", "video", "image"],
                    "description": "新内容类型（可选）"
                },
                "body": { "type": "string", "description": "新正文内容（可选）" },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "新标签列表（可选）"
                },
                "status": {
                    "type": "string",
                    "enum": ["draft", "published"],
                    "description": "新状态（可选）"
                }
            },
            "required": ["id"]
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Integration
    }
    fn domain(&self) -> ToolDomain {
        ToolDomain::Automation
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // #36 批 2：写面走插件命名命令（`opc_update_content_asset`）。工具入参沿用
        // `content_type`（智能体侧历史口径），命令面线格式是 camelCase ⇒ 这一处改名由
        // `filter_request` 承担（禁区 13：跨进程 DTO 用 `contentType`）。缺键 = 该列不动。
        const COMMAND: &str = "opc_update_content_asset";
        let id = input
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("缺少必需参数: id"))?;

        let mut payload = filter_request(
            &input,
            &[
                ("title", "title"),
                ("content_type", "contentType"),
                ("body", "body"),
                ("tags", "tags"),
                ("status", "status"),
            ],
        );
        if let Some(map) = payload.as_object_mut() {
            map.insert("id".to_string(), json!(id));
        }

        let outcome = call_opc_command(ctx, COMMAND, payload)
            .await
            .map_err(|e| ToolError::execution_failed(format!("更新内容资产失败: {e}")))?;
        // 缺行是**业务读数**（不是故障）：明确告诉调用方未做任何更新。
        let Some(asset) = outcome.get("updated") else {
            return Ok(ToolResult::success(format!(
                "## 内容资产不存在\n\n`{id}` 不在内容资产表里，未做任何更新。"
            )));
        };
        Ok(ToolResult::success(format!(
            "## 内容资产已更新\n\n- **标题**: {}\n- **类型**: {}\n- **状态**: {}\n- **ID**: `{}`",
            json_str(asset, "title"),
            json_str(asset, "contentType"),
            json_str(asset, "status"),
            json_str(asset, "id"),
        )))
    }
}

pub struct OpcDeleteContentAssetTool;

#[async_trait]
impl Tool for OpcDeleteContentAssetTool {
    fn name(&self) -> &str {
        "OpcDeleteContentAsset"
    }
    fn description(&self) -> &str {
        "删除指定内容资产。需要提供内容资产ID。"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "内容资产ID" }
            },
            "required": ["id"]
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Integration
    }
    fn domain(&self) -> ToolDomain {
        ToolDomain::Automation
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // #36 批 2：写面走插件命名命令（`opc_delete_content_asset`），出参两态。
        const COMMAND: &str = "opc_delete_content_asset";
        let id = input
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("缺少必需参数: id"))?;

        let outcome = call_opc_command(ctx, COMMAND, json!({ "id": id }))
            .await
            .map_err(|e| ToolError::execution_failed(format!("删除内容资产失败: {e}")))?;
        // 缺行是**业务读数**（不是故障）：没有删掉任何行这件事要如实回报。
        if outcome.as_str() == Some("not_found") {
            return Ok(ToolResult::success(format!(
                "## 内容资产不存在\n\n`{id}` 不在内容资产表里，未做任何删除。"
            )));
        }
        Ok(ToolResult::success(format!("## 内容资产已删除\n\n- **ID**: `{id}`")))
    }
}

// ═══════════════════════════════════════════════════════════════════
// 发布计划工具
// ═══════════════════════════════════════════════════════════════════

pub struct OpcCreatePublishScheduleTool;

#[async_trait]
impl Tool for OpcCreatePublishScheduleTool {
    fn name(&self) -> &str {
        "OpcCreatePublishSchedule"
    }
    fn description(&self) -> &str {
        "创建新的发布计划。需要提供内容引用类型、ID 和计划发布时间。"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content_ref_type": {
                    "type": "string",
                    "enum": ["blog_post", "content_asset"],
                    "description": "内容引用类型"
                },
                "content_ref_id": { "type": "string", "description": "内容引用ID" },
                "scheduled_at": { "type": "integer", "description": "计划发布时间戳" }
            },
            "required": ["content_ref_type", "content_ref_id", "scheduled_at"]
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Integration
    }
    fn domain(&self) -> ToolDomain {
        ToolDomain::Automation
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // #36 批 3：写面走插件命名命令（`opc_create_publish_schedule`）。`content_ref_id`
        // 是**被发布内容**的主键（博客 / 内容资产 id），不是发布计划自己的 id（#180 的教训）。
        const COMMAND: &str = "opc_create_publish_schedule";
        let content_ref_type = input
            .get("content_ref_type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("缺少必需参数: content_ref_type"))?;
        let content_ref_id = input
            .get("content_ref_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("缺少必需参数: content_ref_id"))?;
        let scheduled_at = input
            .get("scheduled_at")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| ToolError::invalid_input("缺少必需参数: scheduled_at"))?;

        let schedule = call_opc_command(
            ctx,
            COMMAND,
            json!({
                "contentRefType": content_ref_type,
                "contentRefId": content_ref_id,
                "scheduledAt": scheduled_at,
            }),
        )
        .await
        .map_err(|e| ToolError::execution_failed(format!("创建发布计划失败: {e}")))?;
        Ok(ToolResult::success(format!(
            "## 发布计划创建成功\n\n- **类型**: {}\n- **计划时间**: {}\n- **状态**: 📋 待发布\n- **ID**: `{}`",
            json_str(&schedule, "contentRefType"),
            json_date(&schedule, "scheduledAt", "%Y-%m-%d %H:%M"),
            json_str(&schedule, "id"),
        )))
    }
}

pub struct OpcListPublishSchedulesTool;

#[async_trait]
impl Tool for OpcListPublishSchedulesTool {
    fn name(&self) -> &str {
        "OpcListPublishSchedules"
    }
    fn description(&self) -> &str {
        "列出所有发布计划。返回内容类型、计划时间、状态等信息。"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{}})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Integration
    }
    fn domain(&self) -> ToolDomain {
        ToolDomain::Automation
    }
    fn is_concurrency_safe(&self) -> bool {
        true
    }
    fn is_read_only(&self) -> bool {
        true
    }

    async fn call(&self, _input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let schedules = call_opc_command(ctx, "opc_list_publish_schedules", json!({}))
            .await
            .map_err(|e| ToolError::execution_failed(format!("查询发布计划失败: {e}")))?;
        let rows = rows_of(schedules);
        if rows.is_empty() {
            return Ok(ToolResult::success("## 发布计划\n\n暂无发布计划。"));
        }
        let lines: Vec<String> = rows
            .iter()
            .map(|s| {
                let status = json_str(s, "status");
                let published = match s.get("publishedAt").and_then(Value::as_i64) {
                    Some(ts) => format!(" | 发布于: {}", format_ts(ts, "%Y-%m-%d %H:%M")),
                    None => String::new(),
                };
                format!(
                    "- **{}** | {} | {} | 计划: {}{}",
                    json_str(s, "contentRefType"),
                    json_str(s, "contentRefId"),
                    if status == "published" {
                        "✅ 已发布"
                    } else if status == "failed" {
                        "❌ 失败"
                    } else {
                        "📋 待发布"
                    },
                    json_date(s, "scheduledAt", "%Y-%m-%d %H:%M"),
                    published,
                )
            })
            .collect();
        Ok(ToolResult::success(format!("## 发布计划 ({} 个)\n\n{}", rows.len(), lines.join("\n"))))
    }
}

pub struct OpcCancelPublishScheduleTool;

#[async_trait]
impl Tool for OpcCancelPublishScheduleTool {
    fn name(&self) -> &str {
        "OpcCancelPublishSchedule"
    }
    fn description(&self) -> &str {
        "取消/删除发布计划。需要提供发布计划ID。"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "发布计划ID" }
            },
            "required": ["id"]
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Integration
    }
    fn domain(&self) -> ToolDomain {
        ToolDomain::Automation
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // #36 批 2：写面走插件命名命令（`opc_cancel_publish_schedule`）。语义是**硬删**
        // （域服务 `delete_publish_schedule` 走 `delete_by_id`），出参两态。
        const COMMAND: &str = "opc_cancel_publish_schedule";
        let id = input
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("缺少必需参数: id"))?;

        let outcome = call_opc_command(ctx, COMMAND, json!({ "id": id }))
            .await
            .map_err(|e| ToolError::execution_failed(format!("取消发布计划失败: {e}")))?;
        // 缺行是**业务读数**（不是故障）：没有删掉任何行这件事要如实回报。
        if outcome.as_str() == Some("not_found") {
            return Ok(ToolResult::success(format!(
                "## 发布计划不存在\n\n`{id}` 不在发布计划表里，未做任何取消。"
            )));
        }
        Ok(ToolResult::success(format!("## 发布计划已取消\n\n- **ID**: `{id}`")))
    }
}

pub struct OpcProcessDueSchedulesTool;

#[async_trait]
impl Tool for OpcProcessDueSchedulesTool {
    fn name(&self) -> &str {
        "OpcProcessDueSchedules"
    }
    fn description(&self) -> &str {
        "处理所有到期的发布计划。将状态为pending且计划时间已到的内容发布出去。"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{}})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Integration
    }
    fn domain(&self) -> ToolDomain {
        ToolDomain::Automation
    }

    async fn call(&self, _input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // #36 批 3：写面走插件命名命令（`opc_process_due_schedules`），出参是本次处理过的
        // 行视图数组；空数组 = 「没有到期计划」，不是错误。
        const COMMAND: &str = "opc_process_due_schedules";
        let results = call_opc_command(ctx, COMMAND, json!({}))
            .await
            .map_err(|e| ToolError::execution_failed(format!("处理发布计划失败: {e}")))?;
        let rows = rows_of(results);
        if rows.is_empty() {
            return Ok(ToolResult::success("## 处理发布计划\n\n暂无到期的发布计划。"));
        }
        let lines: Vec<String> = rows
            .iter()
            .map(|s| {
                format!(
                    "- **{}** | {} | {}",
                    json_str(s, "contentRefType"),
                    json_str(s, "contentRefId"),
                    if json_str(s, "status") == "published" {
                        "✅ 发布成功"
                    } else {
                        "❌ 发布失败"
                    },
                )
            })
            .collect();
        Ok(ToolResult::success(format!(
            "## 处理发布计划 ({} 个)\n\n{}",
            rows.len(),
            lines.join("\n")
        )))
    }
}

// ═══════════════════════════════════════════════════════════════════
// 测试 — blog 工具的插件命令桥契约（#225）
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod blog_bridge_tests {
    use super::*;
    use axagent_harness::error::ToolErrorKind;

    /// 测试替身：按命令名回固定帧，无锁无状态（避开 disallowed-types）。
    #[derive(Debug)]
    struct StubBridge;

    #[async_trait::async_trait]
    impl axagent_harness::PluginCommandBridge for StubBridge {
        async fn execute(
            &self,
            plugin_id: &str,
            command: &str,
            _input: serde_json::Value,
        ) -> Result<serde_json::Value, String> {
            assert_eq!(plugin_id, "opc");
            match command {
                "opc_list_blog_posts" => Ok(serde_json::json!([
                    {
                        "id": "p1",
                        "title": "第一篇",
                        "slug": "first",
                        "excerpt": "",
                        "tags": ["公告", "运营"],
                        "published": true,
                        "publishedAt": null,
                        "viewCount": 7,
                        "createdAt": "2026-01-01T00:00:00Z",
                        "updatedAt": "2026-01-01T00:00:00Z"
                    },
                    {
                        "id": "p2",
                        "title": "第二篇",
                        "slug": "second",
                        "excerpt": "",
                        "tags": [],
                        "published": false,
                        "publishedAt": null,
                        "viewCount": 0,
                        "createdAt": "2026-01-02T00:00:00Z",
                        "updatedAt": "2026-01-02T00:00:00Z"
                    }
                ])),
                "opc_create_blog_post" => Ok(serde_json::json!({
                    "id": "p9",
                    "title": "新建篇",
                    "slug": "new",
                    "excerpt": "",
                    "tags": ["公告"],
                    "published": false,
                    "publishedAt": null,
                    "viewCount": 0,
                    "createdAt": "2026-01-03T00:00:00Z",
                    "updatedAt": "2026-01-03T00:00:00Z"
                })),
                other => Err(format!("替身未实现命令 {other}")),
            }
        }
    }

    #[tokio::test]
    async fn list_fails_closed_without_bridge() {
        let ctx = ToolContext::new("mock");
        let err = OpcListBlogPostsTool
            .call(serde_json::json!({}), &ctx)
            .await
            .expect_err("桥未注入时必须失败");
        assert!(err.message.contains("插件命令桥未注入"), "actual: {}", err.message);
    }

    #[tokio::test]
    async fn list_renders_bridge_rows() {
        let ctx =
            ToolContext::new("mock").with_plugin_command_bridge(std::sync::Arc::new(StubBridge));
        let out = OpcListBlogPostsTool.call(serde_json::json!({}), &ctx).await.expect("应成功");
        assert!(!out.is_error);
        assert!(out.content.contains("## 博客文章 (2 篇)"), "actual: {}", out.content);
        assert!(out.content.contains("**第一篇** (`first`) ✅ 已发布"), "actual: {}", out.content);
        assert!(out.content.contains("阅读: 7"), "actual: {}", out.content);
        assert!(out.content.contains("标签: 公告, 运营"), "actual: {}", out.content);
        assert!(out.content.contains("**第二篇** (`second`) 📝 草稿"), "actual: {}", out.content);
    }

    #[tokio::test]
    async fn create_fails_closed_before_input_validation() {
        // fail-closed 先于输入校验：桥缺失时连「参数不全」都不该告知（不可泄露面顺序）
        let ctx = ToolContext::new("mock");
        let err = OpcCreateBlogPostTool
            .call(serde_json::json!({"slug": "s", "content": "c"}), &ctx)
            .await
            .expect_err("桥未注入时必须失败");
        assert!(err.message.contains("插件命令桥未注入"), "actual: {}", err.message);
    }

    #[tokio::test]
    async fn create_requires_title() {
        let ctx =
            ToolContext::new("mock").with_plugin_command_bridge(std::sync::Arc::new(StubBridge));
        let err = OpcCreateBlogPostTool
            .call(serde_json::json!({"slug": "s", "content": "c"}), &ctx)
            .await
            .expect_err("缺 title 必须失败");
        assert_eq!(err.kind, ToolErrorKind::InvalidInput);
        assert!(err.message.contains("title"), "actual: {}", err.message);
    }

    #[tokio::test]
    async fn create_renders_bridge_frame() {
        let ctx =
            ToolContext::new("mock").with_plugin_command_bridge(std::sync::Arc::new(StubBridge));
        let out = OpcCreateBlogPostTool
            .call(
                serde_json::json!({"title": "新建篇", "slug": "new", "content": "正文", "tags": ["公告"]}),
                &ctx,
            )
            .await
            .expect("应成功");
        assert!(!out.is_error);
        assert!(out.content.contains("**标题**: 新建篇"), "actual: {}", out.content);
        assert!(out.content.contains("**Slug**: `new`"), "actual: {}", out.content);
        assert!(out.content.contains("**状态**: 📝 草稿"), "actual: {}", out.content);
        assert!(out.content.contains("**ID**: `p9`"), "actual: {}", out.content);
    }
}
