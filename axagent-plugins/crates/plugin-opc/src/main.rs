// SPDX-License-Identifier: AGPL-3.0-only
//! OPC 域插件 worker。
//!
//! 宿主以 `worker.program` 相对路径 spawn 本进程，双方在 stdin/stdout 上交换长度前缀
//! JSON 帧（`axagent-plugin-proto`）。诊断信息一律走 stderr —— stdout 是协议通道，
//! 混入任何非帧字节都会让宿主读帧失败。
//!
//! #36 第 1 批只有骨架（describe + 帧循环 + shutdown，零业务命令）。**第 3–4 步**接进第一条
//! 真实链路：命名命令 `opc_list_blog_posts` → 出站只读接缝 `opc.store` 的 `list_blog_posts`
//! op → 宿主 provider → dao。数据主权（连接池、列裁剪、分页闸）全在宿主侧，本进程只做宿主
//! 做不了的一件事：「整表语义」的分页取尽（见 [`blog_posts`]）。出参线格式逐字透传接缝视图
//! 的 camelCase，**不再有任何键名适配层**（禁区 13 无例外，判据见该文「出参线格式」一节）。
//!
//! 为什么用 `Peer` 而不是自写读循环：插件一旦能主动调宿主，同一条管道上就同时挂着
//! 「我方请求的响应」与「对端发来的新请求」，朴素读一帧写一帧必然错配。`Peer` 把对端请求
//! 进 `deferred` 队列，等响应期间不丢帧。
//!
//! 为什么是纯可执行包（只有 `src/main.rs`、没有 `src/lib.rs`）：与 `plugin-stock`、
//! `schema-gen` 同一形态 —— worker 由宿主 spawn，没有 Cargo 消费方，不需要库入口。

use std::io::{StdinLock, StdoutLock};
use std::process::ExitCode;

use axagent_plugin_proto::{
    AXAGENT_PLUGIN_PROTO_VERSION, CommandFault, CommandFrame, CommandOutcome, CommandReply,
    FrameRequest, FrameResponse, Peer, PluginDeclaration, ResponseKind, error_codes, ops, serve,
};
use serde_json::{Value, json};

/// worker 侧的 Peer 具体类型：`serve` 固定用加锁的 stdin / stdout（三处签名共用一份）。
type WorkerPeer = Peer<StdinLock<'static>, StdoutLock<'static>>;

/// 本二进制归属的插件**名**（清单 `manifest.name` 逐字一致）。
///
/// ⚠ 这里存的**不是**完整插件 id：宿主 spawn worker 时经 `AXAGENT_PLUGIN_ID` 注入的是
/// **注册表键**，形态为 `{小写 name}@{marketplace}`（`plugins/src/manager.rs` 的
/// `plugin_id()` + `worker.rs` 的 `command.env("AXAGENT_PLUGIN_ID", …)`）—— 内置包是
/// `opc@bundled`，外部安装是 `opc@external`。⇒ 写死一份完整 id 的字面量在两条路上必然
/// 有一条对不上，会把每一条真实命令都拒成 `PLUGIN_COMMAND_ID_MISMATCH`。
/// 归属判据因此只看**名**：见 [`own_id_belongs_to_this_plugin`]。
const PLUGIN_NAME: &str = "opc";

/// 注入身份是否属于本二进制声明的那个插件（`{name}` 或 `{name}@{任意市场}`）。
fn own_id_belongs_to_this_plugin(own_id: &str) -> bool {
    own_id == PLUGIN_NAME || own_id.starts_with(&format!("{PLUGIN_NAME}@"))
}

/// 出站宿主查询接缝：OPC 站点内容的只读读面（契约见 harness `src/opc_store.rs`）。
/// 本插件不持 `DatabaseConnection`，一切表读都走它。
const OPC_STORE_SEAM: &str = "opc.store";
/// `opc.store` 首批唯一的 op（表外 op 在宿主分派层落 `SEAM_CALL_UNSUPPORTED`）。
const OPC_STORE_LIST_BLOG_POSTS: &str = "list_blog_posts";

/// 本插件承接的命名命令（manifest `commands[].name` 必须与它逐字一致：宿主
/// `execute_plugin_command` 先按清单查命令，未声明的那条直接 `NotFound`）。
const COMMAND_LIST_BLOG_POSTS: &str = "opc_list_blog_posts";

fn main() -> ExitCode {
    match serve(handle) {
        // 对端关管道 = 宿主停机，正常退出（`Peer::run` 里 EOF 不是错误）。
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("[axagent-plugin-opc] 帧通道失败：{e}");
            ExitCode::FAILURE
        },
    }
}

fn handle(request: &FrameRequest, peer: &mut WorkerPeer) -> FrameResponse {
    match request.op.as_str() {
        ops::DESCRIBE => declaration(),
        ops::SHUTDOWN => FrameResponse::success(json!({ "ok": true })),
        // T2 命令面：宿主发 `CommandFrame`（pluginId 绑定 + 命令名 + 入参 + 预算）。
        ops::COMMAND => handle_command(request, peer),
        other => FrameResponse::error(
            error_codes::PLUGIN_UNKNOWN_OP,
            format!("OPC 插件未实现该操作：{other}"),
        ),
    }
}

/// 处理一条命令帧：身份自检 → 按命令名分发。
///
/// 身份自检是**第二道防线**（第一道在宿主 `WorkerInvoker::execute_command` 写帧前）：
/// 帧上的 `plugin_id` 必须等于 spawn 时注入的 `AXAGENT_PLUGIN_ID`，防宿主侧错配。
fn handle_command(request: &FrameRequest, peer: &mut WorkerPeer) -> FrameResponse {
    let frame = match CommandFrame::from_frame_request(request) {
        Ok(frame) => frame,
        Err(e) => {
            return FrameResponse::error(
                error_codes::PLUGIN_COMMAND_INVALID_ARGS,
                format!("命令帧解码失败：{e}"),
            );
        },
    };
    let own_id = std::env::var("AXAGENT_PLUGIN_ID").unwrap_or_default();
    if frame.plugin_id != own_id {
        return CommandFault::for_frame(
            &frame,
            error_codes::PLUGIN_COMMAND_ID_MISMATCH,
            format!("帧绑定 `{}` 与本 worker 身份 `{own_id}` 不符，拒绝执行", frame.plugin_id),
        )
        .into_frame_response();
    }
    // 注入身份与二进制自述身份也要一致：上面那条只在「宿主注入值 = 帧绑定值」时成立，
    // 宿主把 OPC 的帧错配到别的 worker 时两边是**一起错**的 ⇒ 那条拦不住，这里按名再钉一次
    // 二进制归属。判据只看名前缀，不比完整 id（完整 id 有 `@bundled` / `@external` 两种真形态）。
    if !own_id_belongs_to_this_plugin(&own_id) {
        return CommandFault::for_frame(
            &frame,
            error_codes::PLUGIN_COMMAND_ID_MISMATCH,
            format!("worker 身份 `{own_id}` 不属于本二进制声明的插件 `{PLUGIN_NAME}`，拒绝执行"),
        )
        .into_frame_response();
    }
    let outcome = match frame.command.as_str() {
        // 博客列表取数：行数据来自宿主 `opc.store` 的 `list_blog_posts`，本命令只做整表
        // 分页（形状与判据见 [`blog_posts`]）。
        COMMAND_LIST_BLOG_POSTS => match blog_posts(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        other => CommandOutcome::Fault(CommandFault::for_frame(
            &frame,
            error_codes::PLUGIN_COMMAND_UNKNOWN,
            format!("OPC 插件没有命令 `{other}` 的实现"),
        )),
    };
    outcome
        .into_frame_response()
        .unwrap_or_else(|e| FrameResponse::error(error_codes::PLUGIN_COMMAND_FAILED, e.to_string()))
}

/// 握手声明：`calls` 是**出站**准入（宿主 `handle_call_seam` 会校验本插件是否声明过目标接缝），
/// 少写一条 = 运行时被拒。`capabilities` 仍为空：本插件不向宿主提供任何接缝 —— `opc.store`
/// 是「宿主提供给插件」的查询接缝，方向相反，不进 `SUPPORTED_REMOTE_SEAMS`。
fn declaration() -> FrameResponse {
    FrameResponse::success(
        serde_json::to_value(PluginDeclaration {
            proto_version: AXAGENT_PLUGIN_PROTO_VERSION,
            subscribe: Vec::new(),
            calls: vec![OPC_STORE_SEAM.to_string()],
            capabilities: Vec::new(),
        })
        .expect("插件声明是可序列化的纯数据"),
    )
}

/// `opc_list_blog_posts`：博客文章列表，出参是行对象数组。
///
/// ## 为什么要在这里分页取尽
///
/// 它替代的宿主命令一次返回**整表**（同进程 IPC、不过帧、无上限）；接缝侧要过帧，故契约给
/// `limit` 上了区间闸（`OPC_BLOG_POST_LIST_LIMIT_RANGE`）。于是「调用方没指定分页」= 宿主语义
/// 的整表 ⇒ 按接缝回带的 `total` 翻页直到取尽。不这么做的后果不是「少一帧数据」而是**静默少
/// 计数**：`DashboardTab` 的 `blog_posts_count` 取的就是本数组的 `length`，一页封顶会让它把
/// 137 篇显示成 50 篇。
///
/// 给了 `limit` / `offset` 则**原样透传、只取一页**（插件不钳位：越界由宿主按契约区间拒收，
/// 缺省档也交给宿主 ⇒ 本进程里没有第二个 50 / 200，同 `plugin-stock` 的
/// `stock_analysis_records` 判据）。
///
/// ## 出参线格式 = 接缝视图逐字 camelCase（禁区 13，无例外）
///
/// 出参就是接缝视图 `BlogPostView`（契约：harness `src/opc_store.rs`，该结构体带
/// `#[serde(rename_all = "camelCase")]`）的回帧键名，本进程**不做任何键名改写**：
/// `publishedAt` / `viewCount` / `createdAt` / `updatedAt` / `excerpt` / `slug` / `tags` /
/// `title` / `id` / `published`。前端消费方（`src/pages/opc/components/SitesTab.tsx` 的
/// `interface BlogPost` 与表格列 `dataIndex: "viewCount"`）与这套键名逐字对齐 —— 键名在
/// 整条链上只有一套。
///
/// 这里曾有的一层 camelCase → snake_case 逐字段映射（复刻被删宿主 DTO
/// `company-runtime::BlogPost` 的蛇形线格式）已删除 —— 那是禁区 13 全站 camelCase 约定的一处
/// 刻意偏离，迁移初期为了「本批只换通道、不动前端」而临时保留，#167 连同那两个前端消费面
/// 一起收口。新增命令**不许**再建这种出口适配层：接缝给什么就交什么。
///
/// 唯一仍在出口做的形状约束是**字段集合不扩容**：出参逐字 = 接缝回帧里的行对象，接缝哪天
/// 多给一个字段，它会直接出现在出参里 —— 由 `tests/protocol.rs` 与本文件的键集合用例逐字
/// 钉住，扩容属于契约变更，要走评审而不是静默发生。
///
/// ## 少掉的字段
///
/// `content`（博客正文）不过线 —— 裁剪做在 SQL 侧（判据与两个消费方的实证见契约模块头）。
/// 现网无人读列表里的正文；真要读正文走宿主详情面，不属本命令。
fn blog_posts(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    let explicit_limit = input.get("limit").filter(|v| !v.is_null()).cloned();
    let explicit_offset = input.get("offset").filter(|v| !v.is_null()).cloned();

    if explicit_limit.is_some() || explicit_offset.is_some() {
        let mut args = serde_json::Map::new();
        if let Some(limit) = explicit_limit {
            args.insert("limit".to_string(), limit);
        }
        if let Some(offset) = explicit_offset {
            args.insert("offset".to_string(), offset);
        }
        let page = call_list_blog_posts(peer, Value::Object(args), call_chain)?;
        // 逐字透传接缝视图（键名 camelCase，禁区 13）：出口不再有第二套键名。
        return Ok(Value::Array(post_rows_of(&page)));
    }

    // 整表语义：offset 按**实际取回的条数**推进（不是按请求的 limit 推进）——最后一页本来就短。
    let mut rows: Vec<Value> = Vec::new();
    let mut offset: u64 = 0;
    loop {
        let page = call_list_blog_posts(peer, json!({ "offset": offset }), call_chain)?;
        let posts = post_rows_of(&page);
        // 两个终止条件都要：`total` 只在该次调用内自洽（并发写入下可能小于已取行数），
        // 而空页是越界 offset 的合法回帧（契约：空数组不是错误）—— 只看 `total` 会吊死。
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

/// 发一次 `opc.store::list_blog_posts` 帧，把宿主侧的失败落成命令失败。
///
/// 接缝报错 ⇒ **原样透传错误码**，不退化成空列表：空数组在页面上长得和「一篇都没有」一样。
fn call_list_blog_posts(
    peer: &mut WorkerPeer,
    args: Value,
    call_chain: &[String],
) -> Result<Value, (String, String)> {
    let reply = peer
        .call_seam(OPC_STORE_SEAM, OPC_STORE_LIST_BLOG_POSTS, args, call_chain.to_vec())
        .map_err(|e| {
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

/// 接缝回帧里的 `posts` 数组（缺键 / 非数组 ⇒ 空页，**不** panic）。
///
/// 它交出的行**原样**成为命令出参：键名 = 接缝视图的 camelCase，出口不做任何改写
/// （判据见 [`blog_posts`] 的「出参线格式」一节）。
fn post_rows_of(page: &Value) -> Vec<Value> {
    page.get("posts").and_then(Value::as_array).cloned().unwrap_or_default()
}

// ── 测试 ────────────────────────────────────────────────────────────────
//
// 内联测试 mod 追加在文件末尾（`items_after_test_module` 只在 clippy 下暴露）。
//
// 只测两件纯函数事：回帧取列的容错，与「出口零键名改写 + 键集合逐字 camelCase」的形态锁。
// 「真管道上发帧 / 分页取尽」归 `tests/protocol.rs`（需要真进程与真 stdin/stdout）。

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
}
