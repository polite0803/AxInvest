// SPDX-License-Identifier: AGPL-3.0-only
//! OPC 域包 worker 的共享实现。
//!
//! 这里只放两件事：泛型 `opc.store` 接缝 helper，以及由 accounting 域插件承接的
//! 三条命令实现。公共 `plugin-opc` 不再声明这三条命令；其它域包以后按同样形态从本
//! crate 取 helper，禁止把整个 worker 复制一份改名（禁区 12）。
//!
//! 数据主权仍在宿主：本 crate 不持 `DatabaseConnection`，一切读写都经 `call_seam`。

use std::io::{Read, Write};

use axagent_plugin_proto::{Peer, ResponseKind, error_codes};
use serde_json::{Map, Value, json};

/// `opc.store` 是宿主提供给出站接缝；少声明一条 = 运行时被拒。
pub const OPC_STORE_SEAM: &str = "opc.store";

/// 三条会计命令的 seam op 名。
pub const OPC_STORE_LIST_INVOICES: &str = "list_invoices";
pub const OPC_STORE_TRANSITION_INVOICE: &str = "transition_invoice";
pub const OPC_STORE_CREATE_INVOICE: &str = "create_invoice";

/// sales_growth 域包三条命令的 seam op 名。
pub const OPC_STORE_CREATE_CUSTOMER: &str = "create_customer";
pub const OPC_STORE_LIST_LANDING_PAGES: &str = "list_landing_pages";
pub const OPC_STORE_LIST_CONTACTS: &str = "list_contacts";

/// 发票列表的过滤键（camelCase，与契约 `InvoiceListQuery` 的 serde 键一致）。
const QUERY_KEYS_INVOICES: &[&str] = &["status", "customerId", "dateFrom", "dateTo", "limit"];

/// 发一次 `opc.store` 帧，把宿主侧的失败原样落成命令失败。
///
/// 泛型参数不可省：`Peer<R, W>` 的收发端由调用方的 `serve` 决定（stdin/stdout、
/// 测试管道、未来的内存通道都行），把 R/W 写死会让 helper 失去复用价值。
pub fn call_opc_store<R: Read, W: Write>(
    peer: &mut Peer<R, W>,
    op: &str,
    args: Value,
    call_chain: &[String],
) -> Result<Value, (String, String)> {
    let reply = peer.call_seam(OPC_STORE_SEAM, op, args, call_chain.to_vec()).map_err(|e| {
        (error_codes::PLUGIN_COMMAND_FAILED.to_string(), format!("OPC 站点接缝调用失败：{e}"))
    })?;
    match reply.kind {
        ResponseKind::Success => Ok(reply.value.unwrap_or(Value::Null)),
        ResponseKind::Error => Err((
            reply.code.unwrap_or_else(|| error_codes::PLUGIN_COMMAND_FAILED.to_string()),
            reply.message.unwrap_or_else(|| "OPC 站点接缝返回错误但未说明".to_string()),
        )),
    }
}

/// 只读列表 helper：只挑 `keys` 里出现的非 null 键装进 `request`，不补默认值。
pub fn list_rows<R: Read, W: Write>(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<R, W>,
    op: &str,
    keys: &[&str],
) -> Result<Value, (String, String)> {
    if keys.is_empty() {
        return call_opc_store(peer, op, json!({}), call_chain);
    }
    let mut request = Map::new();
    for key in keys {
        if let Some(value) = input.get(*key).filter(|value| !value.is_null()) {
            request.insert((*key).to_string(), value.clone());
        }
    }
    call_opc_store(peer, op, json!({ "request": request }), call_chain)
}

/// `opc_list_invoices`：列出会计域发票。
pub fn list_invoices<R: Read, W: Write>(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<R, W>,
) -> Result<Value, (String, String)> {
    list_rows(input, call_chain, peer, OPC_STORE_LIST_INVOICES, QUERY_KEYS_INVOICES)
}

/// `opc_transition_invoice`：命令面 `status` 在出站帧改名为契约的 `target`。
pub fn transition_invoice<R: Read, W: Write>(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<R, W>,
) -> Result<Value, (String, String)> {
    let id = require_str(input, "opc_transition_invoice", "id")?;
    let status = require_str(input, "opc_transition_invoice", "status")?;
    call_opc_store(
        peer,
        OPC_STORE_TRANSITION_INVOICE,
        json!({ "id": id, "target": status }),
        call_chain,
    )
}

/// `opc_create_invoice`：创建发票。行金额由宿主按 `quantity × unitPrice × (1 + taxRate)` 现算。
pub fn create_invoice<R: Read, W: Write>(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<R, W>,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_create_invoice";
    let customer_id = require_str(input, OP, "customerId")?;
    let line_items = object_array(input, OP, "lineItems")?;
    let mut request = Map::new();
    request.insert("customerId".to_string(), Value::String(customer_id.to_string()));
    request.insert("lineItems".to_string(), line_items);
    copy_optional_strings(input, OP, &mut request, &["currency", "notes"])?;
    copy_optional_i64(input, OP, &mut request, &["dueAt"])?;
    call_opc_store(peer, OPC_STORE_CREATE_INVOICE, json!({ "request": request }), call_chain)
}

/// `opc_create_customer`：创建客户（写 op）。
pub fn create_customer<R: Read, W: Write>(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<R, W>,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_create_customer";
    let name = require_str(input, OP, "name")?;
    let email = require_str(input, OP, "email")?;
    let mut request = Map::new();
    request.insert("name".to_string(), Value::String(name.to_string()));
    request.insert("email".to_string(), Value::String(email.to_string()));
    copy_optional_strings(
        input,
        OP,
        &mut request,
        &[
            "phone",
            "company",
            "customerType",
            "country",
            "region",
            "city",
            "address",
            "source",
            "notes",
        ],
    )?;
    copy_optional_f64(input, OP, &mut request, &["latitude", "longitude"])?;
    call_opc_store(peer, OPC_STORE_CREATE_CUSTOMER, json!({ "request": request }), call_chain)
}

/// `opc_list_landing_pages`：落地页列表（无入参，裸数组透传）。
pub fn list_landing_pages<R: Read, W: Write>(
    _input: &Value,
    call_chain: &[String],
    peer: &mut Peer<R, W>,
) -> Result<Value, (String, String)> {
    list_rows(&json!({}), call_chain, peer, OPC_STORE_LIST_LANDING_PAGES, &[])
}

/// `opc_list_contacts`：联系人列表（无入参，裸数组透传）。
pub fn list_contacts<R: Read, W: Write>(
    _input: &Value,
    call_chain: &[String],
    peer: &mut Peer<R, W>,
) -> Result<Value, (String, String)> {
    list_rows(&json!({}), call_chain, peer, OPC_STORE_LIST_CONTACTS, &[])
}

fn invalid_shape(message: String) -> (String, String) {
    (error_codes::PLUGIN_COMMAND_INVALID_ARGS.to_string(), message)
}

fn require_str<'a>(input: &'a Value, op: &str, key: &str) -> Result<&'a str, (String, String)> {
    input
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_shape(format!("{op} 需要字符串字段 `{key}`（形状判在命令层）")))
}

fn object_array(input: &Value, op: &str, key: &str) -> Result<Value, (String, String)> {
    let items = input
        .get(key)
        .and_then(Value::as_array)
        .filter(|items| items.iter().all(Value::is_object))
        .ok_or_else(|| invalid_shape(format!("{op} 需要对象数组字段 `{key}`（形状判在命令层）")))?;
    Ok(Value::Array(items.clone()))
}

fn copy_optional_strings(
    input: &Value,
    op: &str,
    request: &mut Map<String, Value>,
    keys: &[&str],
) -> Result<(), (String, String)> {
    for key in keys {
        match input.get(*key) {
            None | Some(Value::Null) => {},
            Some(Value::String(value)) => {
                request.insert((*key).to_string(), Value::String(value.clone()));
            },
            Some(_) => {
                return Err(invalid_shape(format!(
                    "{op} 的 `{key}` 必须是字符串（形状判在命令层）"
                )));
            },
        }
    }
    Ok(())
}

fn copy_optional_f64(
    input: &Value,
    op: &str,
    request: &mut Map<String, Value>,
    keys: &[&str],
) -> Result<(), (String, String)> {
    for key in keys {
        match input.get(*key) {
            None | Some(Value::Null) => {},
            Some(value) if value.as_f64().is_some() => {
                request.insert((*key).to_string(), value.clone());
            },
            Some(_) => {
                return Err(invalid_shape(format!("{op} 的 `{key}` 必须是数值（形状判在命令层）")));
            },
        }
    }
    Ok(())
}

fn copy_optional_i64(
    input: &Value,
    op: &str,
    request: &mut Map<String, Value>,
    keys: &[&str],
) -> Result<(), (String, String)> {
    for key in keys {
        match input.get(*key) {
            None | Some(Value::Null) => {},
            Some(value) if value.as_i64().is_some() => {
                request.insert((*key).to_string(), value.clone());
            },
            Some(_) => {
                return Err(invalid_shape(format!(
                    "{op} 的 `{key}` 必须是整数时间戳（形状判在命令层）"
                )));
            },
        }
    }
    Ok(())
}
