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

/// content_media 域包 worker 使用的 seam op 名。
pub const OPC_STORE_LIST_BLOG_POSTS: &str = "list_blog_posts";
pub const OPC_STORE_PUBLISH_BLOG_POST: &str = "publish_blog_post";
pub const OPC_STORE_DELETE_BLOG_POST: &str = "delete_blog_post";
pub const OPC_STORE_CREATE_BLOG_POST: &str = "create_blog_post";
pub const OPC_STORE_LIST_CONTENT_ASSETS: &str = "list_content_assets";
pub const OPC_STORE_LIST_PUBLISH_SCHEDULES: &str = "list_publish_schedules";
pub const OPC_STORE_CREATE_CONTENT_ASSET: &str = "create_content_asset";
pub const OPC_STORE_CREATE_PUBLISH_SCHEDULE: &str = "create_publish_schedule";
pub const OPC_STORE_UPDATE_CONTENT_ASSET: &str = "update_content_asset";
pub const OPC_STORE_DELETE_CONTENT_ASSET: &str = "delete_content_asset";
pub const OPC_STORE_CANCEL_PUBLISH_SCHEDULE: &str = "cancel_publish_schedule";
pub const OPC_STORE_PROCESS_DUE_SCHEDULES: &str = "process_due_schedules";

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

/// 接缝回帧里的 `posts` 数组（缺键 / 非数组 ⇒ 空页）。
pub fn post_rows_of(page: &Value) -> Vec<Value> {
    page.get("posts").and_then(Value::as_array).cloned().unwrap_or_default()
}

/// `opc_list_blog_posts`：无分页时按实际取回条数推进，取尽整表。
pub fn blog_posts<R: Read, W: Write>(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<R, W>,
) -> Result<Value, (String, String)> {
    let explicit_limit = input.get("limit").filter(|v| !v.is_null()).cloned();
    let explicit_offset = input.get("offset").filter(|v| !v.is_null()).cloned();
    if explicit_limit.is_some() || explicit_offset.is_some() {
        let mut args = Map::new();
        if let Some(limit) = explicit_limit {
            args.insert("limit".to_string(), limit);
        }
        if let Some(offset) = explicit_offset {
            args.insert("offset".to_string(), offset);
        }
        let page =
            call_opc_store(peer, OPC_STORE_LIST_BLOG_POSTS, Value::Object(args), call_chain)?;
        return Ok(Value::Array(post_rows_of(&page)));
    }
    let mut rows: Vec<Value> = Vec::new();
    let mut offset: u64 = 0;
    loop {
        let page = call_opc_store(
            peer,
            OPC_STORE_LIST_BLOG_POSTS,
            json!({ "offset": offset }),
            call_chain,
        )?;
        let posts = post_rows_of(&page);
        let total = page.get("total").and_then(Value::as_u64).unwrap_or(0);
        let fetched = posts.len();
        rows.extend(posts);
        offset += fetched as u64;
        if fetched == 0 || rows.len() as u64 >= total {
            break;
        }
    }
    Ok(Value::Array(rows))
}

/// `opc_publish_blog_post`：两态枚举串逐字透传。
pub fn publish_blog_post<R: Read, W: Write>(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<R, W>,
) -> Result<Value, (String, String)> {
    let id = require_str(input, "opc_publish_blog_post", "id")?;
    call_opc_store(peer, OPC_STORE_PUBLISH_BLOG_POST, json!({ "id": id }), call_chain)
}

/// `opc_delete_blog_post`：三态枚举串逐字透传。
pub fn delete_blog_post<R: Read, W: Write>(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<R, W>,
) -> Result<Value, (String, String)> {
    let id = require_str(input, "opc_delete_blog_post", "id")?;
    call_opc_store(peer, OPC_STORE_DELETE_BLOG_POST, json!({ "id": id }), call_chain)
}

/// `opc_create_blog_post`：五个平铺字段，嵌 request 一层。
pub fn create_blog_post<R: Read, W: Write>(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<R, W>,
) -> Result<Value, (String, String)> {
    let request = blog_post_create_request(input)?;
    call_opc_store(peer, OPC_STORE_CREATE_BLOG_POST, json!({ "request": request }), call_chain)
}

/// `opc_create_content_asset`：创建内容资产（写 op，创建后为草稿态）。
pub fn create_content_asset<R: Read, W: Write>(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<R, W>,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_create_content_asset";
    let title = require_str(input, OP, "title")?;
    let content_type = require_str(input, OP, "contentType")?;
    let body = require_str(input, OP, "body")?;
    let mut request = Map::new();
    request.insert("title".to_string(), Value::String(title.to_string()));
    request.insert("contentType".to_string(), Value::String(content_type.to_string()));
    request.insert("body".to_string(), Value::String(body.to_string()));
    if let Some(tags) = input.get("tags").filter(|value| !value.is_null()) {
        let items = tags.as_array().filter(|items| items.iter().all(Value::is_string)).ok_or_else(
            || invalid_shape(format!("{OP} 的 `tags` 必须是字符串数组（形状判在命令层）")),
        )?;
        request.insert("tags".to_string(), Value::Array(items.clone()));
    }
    call_opc_store(
        peer,
        OPC_STORE_CREATE_CONTENT_ASSET,
        json!({ "request": Value::Object(request) }),
        call_chain,
    )
}

/// `opc_create_publish_schedule`：创建发布计划（写 op）。
pub fn create_publish_schedule<R: Read, W: Write>(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<R, W>,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_create_publish_schedule";
    let content_ref_type = require_str(input, OP, "contentRefType")?;
    let content_ref_id = require_str(input, OP, "contentRefId")?;
    let scheduled_at = input
        .get("scheduledAt")
        .and_then(Value::as_i64)
        .ok_or_else(|| invalid_shape(format!("{OP} 需要整数时间戳字段 `scheduledAt`")))?;
    call_opc_store(
        peer,
        OPC_STORE_CREATE_PUBLISH_SCHEDULE,
        json!({
            "request": {
                "contentRefType": content_ref_type,
                "contentRefId": content_ref_id,
                "scheduledAt": scheduled_at,
            }
        }),
        call_chain,
    )
}

fn blog_post_create_request(input: &Value) -> Result<Value, (String, String)> {
    for key in ["title", "slug", "excerpt", "content"] {
        if input.get(key).and_then(Value::as_str).is_none() {
            return Err(invalid_shape(format!(
                "opc_create_blog_post 需要平铺的字符串字段 `{key}`"
            )));
        }
    }
    let tags = match input.get("tags") {
        Some(Value::Array(items)) if items.iter().all(Value::is_string) => {
            Value::Array(items.clone())
        },
        _ => {
            return Err(invalid_shape(
                "opc_create_blog_post 的 `tags` 必须是字符串数组".to_string(),
            ));
        },
    };
    Ok(json!({
        "title": input["title"], "slug": input["slug"], "excerpt": input["excerpt"],
        "content": input["content"], "tags": tags,
    }))
}

/// `opc_update_content_asset`：id + 五个可选字段。
pub fn update_content_asset<R: Read, W: Write>(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<R, W>,
) -> Result<Value, (String, String)> {
    let id = require_str(input, "opc_update_content_asset", "id")?;
    call_opc_store(
        peer,
        OPC_STORE_UPDATE_CONTENT_ASSET,
        json!({ "id": id, "request": content_asset_update_request(input)? }),
        call_chain,
    )
}

fn content_asset_update_request(input: &Value) -> Result<Value, (String, String)> {
    let mut request = Map::new();
    for key in ["title", "contentType", "body", "status"] {
        match input.get(key) {
            None | Some(Value::Null) => {},
            Some(Value::String(value)) => {
                request.insert(key.to_string(), Value::String(value.clone()));
            },
            Some(_) => {
                return Err(invalid_shape(format!(
                    "opc_update_content_asset 的 `{key}` 必须是字符串"
                )));
            },
        }
    }
    match input.get("tags") {
        None | Some(Value::Null) => {},
        Some(Value::Array(items)) if items.iter().all(Value::is_string) => {
            request.insert("tags".to_string(), Value::Array(items.clone()));
        },
        _ => {
            return Err(invalid_shape(
                "opc_update_content_asset 的 `tags` 必须是字符串数组".to_string(),
            ));
        },
    }
    Ok(Value::Object(request))
}

/// `opc_delete_content_asset` / `opc_cancel_publish_schedule` 的共享 id 形状闸。
pub fn id_command<R: Read, W: Write>(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<R, W>,
    op_name: &str,
    seam_op: &str,
) -> Result<Value, (String, String)> {
    let id = require_str(input, op_name, "id")?;
    call_opc_store(peer, seam_op, json!({ "id": id }), call_chain)
}

/// 列表 helper：无入参裸数组透传。
pub fn list_rows_no_args<R: Read, W: Write>(
    call_chain: &[String],
    peer: &mut Peer<R, W>,
    seam_op: &str,
) -> Result<Value, (String, String)> {
    call_opc_store(peer, seam_op, json!({}), call_chain)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn seam_row() -> Value {
        json!({
            "id": "post-1",
            "title": "第一篇",
            "slug": "first-post",
            "excerpt": "摘要",
            "tags": ["opc", "blog"],
            "published": true,
            "publishedAt": 1_700_000_000_000_i64,
            "viewCount": 42,
            "createdAt": 1_700_000_000_000_i64,
            "updatedAt": 1_700_000_060_000_i64,
        })
    }

    /// 出口的「零键名改写」形态锁：命令出参逐字 = 接缝回帧里的行，键集合是那 10 个 camelCase
    /// 键（禁区 13）。前端表格列按 `dataIndex` 取值，键名漂一个字母不会报错，只会让那一列
    /// 静默变空 ⇒ 这是本命令唯一的形态锁。
    #[test]
    fn rows_pass_through_verbatim_with_exactly_the_camel_case_keys() {
        let rows = post_rows_of(&json!({ "posts": [seam_row()] }));
        assert_eq!(rows.len(), 1);
        let row = &rows[0];

        // ① 出口没有适配层：交出的行与接缝回帧逐字节相同。
        assert_eq!(*row, seam_row(), "出参必须逐字透传接缝视图，不得改写键名");

        // ② 键集合逐字钉住：接缝扩容新字段要连同前端一起走评审，不许静默出现在出参里。
        let mut keys: Vec<String> = row.as_object().expect("行应是对象").keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "createdAt",
                "excerpt",
                "id",
                "published",
                "publishedAt",
                "slug",
                "tags",
                "title",
                "updatedAt",
                "viewCount"
            ]
        );

        // ③ snake_case 改写与正文列一律不许出现。
        for bad in ["published_at", "view_count", "created_at", "updated_at", "content"] {
            assert!(row.get(bad).is_none(), "出口不得再造 snake_case / 正文列键：{bad}");
        }
        assert_eq!(row["viewCount"], json!(42));
        assert_eq!(row["publishedAt"], json!(1_700_000_000_000_i64));
        assert_eq!(row["tags"], json!(["opc", "blog"]));
    }

    /// 草稿（`publishedAt = null`）必须透出 `null` 且**保留键**：契约里它是 `Option<i64>`，
    /// 序列化出的就是 `null`；前端把「值为 null」和「键不存在」当成两种东西（后者读出来是
    /// `undefined`，会落进缺省分支）。
    #[test]
    fn null_stays_null_and_key_still_present() {
        let mut row = seam_row();
        row["publishedAt"] = json!(null);
        row["published"] = json!(false);
        let rows = post_rows_of(&json!({ "posts": [row] }));
        assert_eq!(rows[0]["publishedAt"], json!(null));
        assert!(rows[0].get("publishedAt").is_some(), "键必须存在");
        assert_eq!(rows[0]["published"], json!(false));
    }

    #[test]
    fn posts_absent_or_wrong_type_yields_empty_page() {
        for page in [json!({}), json!({ "posts": null }), json!({ "posts": {} })] {
            assert!(post_rows_of(&page).is_empty(), "缺键 / 畸形 posts 都当空页：{page}");
        }
        let page = json!({ "posts": [seam_row()] });
        assert_eq!(post_rows_of(&page).len(), 1);
    }

    /// 创建命令的**形状闸**：五个字段齐 / 类型对才放行，且只转发这五个键。
    ///
    /// 另两档同样要钉：① 空串与超预算正文**照常放行**（语义闸在宿主 provider，本层越界就会
    /// 变成「两处各判一半」）；② 缺字段 / 类型不符**一律拒**（不发帧）。
    #[test]
    fn create_request_shape_gate_keeps_exactly_five_keys() {
        let ok = blog_post_create_request(&json!({
            "title": "标题",
            "slug": "first-post",
            "excerpt": "摘要",
            "content": "正文",
            "tags": ["opc", "blog"],
            // 多余键：不转发（宿主 serde 本来也会忽略未知字段，本层给它同一条口径）。
            "extra": 1,
        }))
        .expect("五个字段齐应放行");
        let mut keys: Vec<String> = ok.as_object().expect("应是对象").keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            ["content", "excerpt", "slug", "tags", "title"],
            "只转发这五个键（接缝 args 的形状是契约）：{ok}"
        );
        assert_eq!(ok["tags"], json!(["opc", "blog"]));

        // 语义闸不在本层：空 slug / 超预算正文都放行（宿主 provider 会拒）。
        let semantic = blog_post_create_request(&json!({
            "title": "",
            "slug": "   ",
            "excerpt": "",
            "content": "x".repeat(70 * 1024),
            "tags": [],
        }));
        assert!(semantic.is_ok(), "空 slug / 超预算必须留给宿主判（两处各判一半必然漂移）");

        // 形状不符：缺字段、类型不符、tags 非字符串数组 —— 一律拒。
        for bad in [
            json!({}),
            json!({ "title": 1, "slug": "s", "excerpt": "e", "content": "c", "tags": [] }),
            json!({ "title": "t", "slug": "s", "content": "c", "tags": [] }),
            json!({ "title": "t", "slug": "s", "excerpt": "e", "content": "c", "tags": "opc" }),
            json!({ "title": "t", "slug": "s", "excerpt": "e", "content": "c", "tags": [1] }),
        ] {
            let (code, message) = blog_post_create_request(&bad).expect_err("形状不符必须拒");
            assert_eq!(code, error_codes::PLUGIN_COMMAND_INVALID_ARGS, "错误码必须逐字：{message}");
            assert!(!message.is_empty(), "错误文案必须说明缺什么");
        }
    }
}
