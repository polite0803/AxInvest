// SPDX-License-Identifier: AGPL-3.0-only
//! OPC 域插件 worker。
//!
//! 宿主以 `worker.program` 相对路径 spawn 本进程，双方在 stdin/stdout 上交换长度前缀
//! JSON 帧（`axagent-plugin-proto`）。诊断信息一律走 stderr —— stdout 是协议通道，
//! 混入任何非帧字节都会让宿主读帧失败。
//!
//! #36 第 1 批只有骨架（describe + 帧循环 + shutdown，零业务命令）。**第 3–4 步**接进第一条
//! 真实链路：命名命令 `opc_list_blog_posts` → 出站只读接缝 `opc.store` 的 `list_blog_posts`
//! op → 宿主 provider → dao。**写面第一批**加 `opc_publish_blog_post` → 同一接缝的
//! `publish_blog_post` op（两态枚举逐字透传）。**本批（#180-①②）**再加两条写命令：
//! `opc_delete_blog_post` → `delete_blog_post`（**三态**：`removed` / `not_found` /
//! `protected`）与 `opc_create_blog_post` → `create_blog_post`（入参是既有
//! `CreateBlogPostInput` 的五个字段，正文的**字节预算**在宿主判、本进程一个数字都不写）。
//! 四条命令的数据主权（连接池、列裁剪、分页闸、写列清单、字节闸、保护 id 名单）全在宿主侧，
//! 本进程只做宿主做不了的一件事：「整表语义」的分页取尽（见 [`blog_posts`]）。出参线格式
//! 逐字透传接缝视图 / 枚举串的 camelCase 与 snake_case，**不再有任何键名适配层**
//! （禁区 13 无例外，判据见该文「出参线格式」一节）。
//!
//! **本批（#36 批 1）**再加 9 条只读命令：8 条 `opc_list_*` 各对一条 `opc.store` 只读 op
//! （出参 = 接缝回帧的裸数组逐字透传），`opc_get_dashboard` 由本进程组合三次列表查询现算
//! （宿主不新增聚合 op）。
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

/// 出站宿主接缝：OPC 站点内容的数据面（契约见 harness `src/opc_store.rs`）。
/// 本插件不持 `DatabaseConnection`，一切表读表写都走它。
const OPC_STORE_SEAM: &str = "opc.store";
/// 读 op（表外 op 在宿主分派层落 `SEAM_CALL_UNSUPPORTED`）。
const OPC_STORE_LIST_BLOG_POSTS: &str = "list_blog_posts";
/// 写 op：把一行博客置为已发布。宿主侧要 `write` 权限（`worker.rs` 的
/// `required_permission_for`），故**本插件清单必须声明 `write`** —— `subprocess_execution`
/// 补不齐它（`has_permission` 是精确 `contains`），少声明一条就是 `PLUGIN_PERMISSION_DENIED`。
const OPC_STORE_PUBLISH_BLOG_POST: &str = "publish_blog_post";
/// 写 op：删除一行博客（同样要 `write`）。出参是**三态**枚举串（`removed` / `not_found` /
/// `protected`），与 `publish_blog_post` 的两态是同一套「正常值走 payload」口径。
const OPC_STORE_DELETE_BLOG_POST: &str = "delete_blog_post";
/// 写 op：新建一行博客（同样要 `write`）。正文的字节预算由**宿主**按契约常量判，
/// 本进程既不截断也不预算（`MAX_FRAME_LEN` 那 64 MiB 是帧层上限，不是本 op 的预算）。
const OPC_STORE_CREATE_BLOG_POST: &str = "create_blog_post";

/// 八个只读查询 op（#36 批 1，均为读面 ⇒ 宿主侧**无权限闸**）：入参是嵌一层的
/// `request` DTO（契约 `*ListQuery`），出参是 `*SummaryView` 的**裸数组**。
const OPC_STORE_LIST_INVOICES: &str = "list_invoices";
const OPC_STORE_LIST_CUSTOMERS: &str = "list_customers";
const OPC_STORE_LIST_PROJECTS: &str = "list_projects";
const OPC_STORE_LIST_LANDING_PAGES: &str = "list_landing_pages";
const OPC_STORE_LIST_CONTACTS: &str = "list_contacts";
const OPC_STORE_LIST_KPIS: &str = "list_kpis";
const OPC_STORE_LIST_CONTENT_ASSETS: &str = "list_content_assets";
const OPC_STORE_LIST_PUBLISH_SCHEDULES: &str = "list_publish_schedules";

/// 本插件承接的命名命令（manifest `commands[].name` 必须与它逐字一致：宿主
/// `execute_plugin_command` 先按清单查命令，未声明的那条直接 `NotFound`）。
const COMMAND_LIST_BLOG_POSTS: &str = "opc_list_blog_posts";
/// 发布命令。宿主同名 Tauri 命令已在 #36 blog 刀删除（先例 #144 的双声明收口：
/// 前端已切完通道）——本条是发布博客的唯一通道，回宿主的两态枚举串。
const COMMAND_PUBLISH_BLOG_POST: &str = "opc_publish_blog_post";
/// 删除命令。宿主同名 Tauri 命令已随 #36 blog 刀删除 —— 本条是删除博客的唯一通道，
/// 出参是三态枚举串逐字透传。
const COMMAND_DELETE_BLOG_POST: &str = "opc_delete_blog_post";
/// 新建命令。宿主同名 Tauri 命令已随 #36 blog 刀删除 —— 本条是新建博客的唯一通道。
///
/// ⚠ 入参形状与已删的宿主命令**不同名不同形**：宿主命令曾是 `{ input: CreateBlogPostInput }`
/// （Tauri 具名参数），本命令按清单 `inputSchema` 收**平铺**的五个字段（与 `opc_publish_blog_post`
/// 的平铺 `id` 同风格）。
const COMMAND_CREATE_BLOG_POST: &str = "opc_create_blog_post";

/// #36 批 1 的 9 条只读命令：前 8 条各对一条只读接缝 op，第 9 条由本进程组合三次列表调用
/// 现算看板读数（宿主**不**新增聚合 op）。
const COMMAND_LIST_INVOICES: &str = "opc_list_invoices";
const COMMAND_LIST_CUSTOMERS: &str = "opc_list_customers";
const COMMAND_LIST_PROJECTS: &str = "opc_list_projects";
const COMMAND_LIST_LANDING_PAGES: &str = "opc_list_landing_pages";
const COMMAND_LIST_CONTACTS: &str = "opc_list_contacts";
const COMMAND_LIST_KPIS: &str = "opc_list_kpis";
const COMMAND_LIST_CONTENT_ASSETS: &str = "opc_list_content_assets";
const COMMAND_LIST_PUBLISH_SCHEDULES: &str = "opc_list_publish_schedules";
const COMMAND_GET_DASHBOARD: &str = "opc_get_dashboard";

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
        // 发布一篇博客：唯一写面，本命令只做入参形状 + 逐字透出宿主的两态结果。
        COMMAND_PUBLISH_BLOG_POST => {
            match publish_blog_post(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // 删除一篇博客：入参形状同 publish（只有 `id`），出参是**三态**枚举的逐字透传
        // （判据见 [`delete_blog_post`]）。
        COMMAND_DELETE_BLOG_POST => {
            match delete_blog_post(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // 新建一篇博客：入参是平铺的五个 DTO 字段（形状判在本层），出参是列裁剪视图
        // （判据见 [`create_blog_post`]）。
        COMMAND_CREATE_BLOG_POST => {
            match create_blog_post(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // ── #36 批 1：八条只读查询命令（出参 = 接缝回帧的裸数组，逐字透传）──
        COMMAND_LIST_INVOICES => match list_rows(
            &frame.input,
            &request.call_chain,
            peer,
            OPC_STORE_LIST_INVOICES,
            QUERY_KEYS_INVOICES,
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_LIST_CUSTOMERS => match list_rows(
            &frame.input,
            &request.call_chain,
            peer,
            OPC_STORE_LIST_CUSTOMERS,
            QUERY_KEYS_CUSTOMERS,
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_LIST_PROJECTS => match list_rows(
            &frame.input,
            &request.call_chain,
            peer,
            OPC_STORE_LIST_PROJECTS,
            QUERY_KEYS_PROJECTS,
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // 以下四条无入参（契约 op 也不收 query）。
        COMMAND_LIST_LANDING_PAGES => {
            match list_rows(
                &frame.input,
                &request.call_chain,
                peer,
                OPC_STORE_LIST_LANDING_PAGES,
                &[],
            ) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_LIST_CONTACTS => {
            match list_rows(&frame.input, &request.call_chain, peer, OPC_STORE_LIST_CONTACTS, &[]) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_LIST_KPIS => match list_rows(
            &frame.input,
            &request.call_chain,
            peer,
            OPC_STORE_LIST_KPIS,
            QUERY_KEYS_KPIS,
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_LIST_CONTENT_ASSETS => {
            match list_rows(
                &frame.input,
                &request.call_chain,
                peer,
                OPC_STORE_LIST_CONTENT_ASSETS,
                &[],
            ) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_LIST_PUBLISH_SCHEDULES => {
            match list_rows(
                &frame.input,
                &request.call_chain,
                peer,
                OPC_STORE_LIST_PUBLISH_SCHEDULES,
                &[],
            ) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // 看板读数：宿主没有聚合 op，由本进程组合三次列表查询现算。
        COMMAND_GET_DASHBOARD => match dashboard(&request.call_chain, peer) {
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
        let page =
            call_opc_store(peer, OPC_STORE_LIST_BLOG_POSTS, Value::Object(args), call_chain)?;
        // 逐字透传接缝视图（键名 camelCase，禁区 13）：出口不再有第二套键名。
        return Ok(Value::Array(post_rows_of(&page)));
    }

    // 整表语义：offset 按**实际取回的条数**推进（不是按请求的 limit 推进）——最后一页本来就短。
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

/// 发一次 `opc.store` 的帧（op 由调用方给），把宿主侧的失败落成命令失败。
///
/// 接缝报错 ⇒ **原样透传错误码**，不退化成空列表 / 假成功：空数组在页面上长得和
/// 「一篇都没有」一样，而「发布失败」被吞掉更直接 —— 用户看见「已发布」但库里没动。
///
/// 读面与写面共用这一条（只差 `op` 与 `args` 两个参数）：两份「解 `ResponseKind`、
/// 缺码兜底、缺文案兜底」的实现对齐在同一处才不会漂移（禁区 12）。
fn call_opc_store(
    peer: &mut WorkerPeer,
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

/// `opc_publish_blog_post`：发布一篇博客（本插件的写面之一）。
///
/// ## 入参只有 `id`，且只判形状
///
/// 命令层只看 `id` 在不在、是不是字符串（不是 ⇒ `PLUGIN_COMMAND_INVALID_ARGS`，**不发帧**）；
/// 「非空」这条语义闸留在宿主 provider（`opc_store_service.rs` 的 `require_publish_id`）。
/// 分两层各判一半会漂移，故刻意分工成「形状 / 语义」而不是两处都判空白。
/// 正文 `content` 从来不是本命令的入参 —— 接缝那侧也没读它。
///
/// ## 出参 = 接缝的两态枚举串，逐字透传
///
/// `"published"` / `"not_found"`（契约：`PublishBlogPostOutcome`，`#[serde(rename_all =
/// "snake_case")]`）。这里**不建包装对象、不改取值、不建适配层**（禁区 13 那条判据同样
/// 管枚举）：「这一篇不在表里」是**正常结果**，插件侧不许把它翻成错误码 —— 翻了就等于让
/// 前端分不出「发布失败」与「本来就没有这篇」。
fn publish_blog_post(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    let Some(id) = input.get("id").and_then(Value::as_str) else {
        return Err((
            error_codes::PLUGIN_COMMAND_INVALID_ARGS.to_string(),
            "opc_publish_blog_post 需要字符串 id（形状判在命令层，非空判在宿主 provider）"
                .to_string(),
        ));
    };
    // 出站帧的 args 只有一个键 `id` —— 逐字与接缝 op 的入参同名，不套 `request` 外层
    // （那是 `knowledge.vector` / `stock.monitor` 的嵌套 DTO 形态；这里没有 DTO）。
    call_opc_store(peer, OPC_STORE_PUBLISH_BLOG_POST, json!({ "id": id }), call_chain)
}

/// 接缝回帧里的 `posts` 数组（缺键 / 非数组 ⇒ 空页，**不** panic）。
///
/// 它交出的行**原样**成为命令出参：键名 = 接缝视图的 camelCase，出口不做任何改写
/// （判据见 [`blog_posts`] 的「出参线格式」一节）。
fn post_rows_of(page: &Value) -> Vec<Value> {
    page.get("posts").and_then(Value::as_array).cloned().unwrap_or_default()
}

/// `opc_delete_blog_post`：删除一篇博客（#180-① 的写 op）。
///
/// ## 入参只有 `id`，且只判形状
///
/// 与 [`publish_blog_post`] 逐条同构：命令层只看 `id` 在不在、是不是字符串
/// （不是 ⇒ `PLUGIN_COMMAND_INVALID_ARGS`，**不发帧**）；「非空」这条语义闸留在宿主
/// provider（`init/opc_store_service.rs` 的 `require_blog_post_id`）。正文不是入参。
///
/// ## 出参 = 接缝的三态枚举串，逐字透传
///
/// `"removed"` / `"not_found"` / `"protected"`（契约：`RemoveBlogPostOutcome`，
/// `#[serde(rename_all = "snake_case")]`，与 `jobs.store` 的 `remove` 同一套取值）。
/// **不建包装对象、不改取值、不建适配层**（禁区 13 那条判据同样管枚举）：
/// 「本来就没有这篇」与「这篇受保护、不许删」都是**正常结果**，插件侧不许把它们翻成错误码 ——
/// 翻了就等于让前端分不出「删失败」「本来没有」「不许删」三件事。
fn delete_blog_post(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    let Some(id) = input.get("id").and_then(Value::as_str) else {
        return Err((
            error_codes::PLUGIN_COMMAND_INVALID_ARGS.to_string(),
            "opc_delete_blog_post 需要字符串 id（形状判在命令层，非空判在宿主 provider）"
                .to_string(),
        ));
    };
    // 出站帧的 args 只有一个键 `id` —— 与接缝 op 的入参同名，不套 `request` 外层。
    call_opc_store(peer, OPC_STORE_DELETE_BLOG_POST, json!({ "id": id }), call_chain)
}

/// `opc_create_blog_post`：新建一篇博客（#180-② 的写 op）。
///
/// ## 入参形状：平铺的五个 DTO 字段（判形状，不判语义）
///
/// 清单 `inputSchema` 声明 `title` / `slug` / `excerpt` / `content` / `tags` 五个属性，
/// 本层只验**形状**（前四个是字符串、`tags` 是字符串数组）。语义闸（`slug` 空、正文超
/// 字节预算）全在宿主 provider —— 两处各判一半必然漂移，且字节预算的数字是对接缝的约定，
/// 不该在进程侧再抄一份（`MAX_FRAME_LEN` 的 64 MiB 是帧层上限，**不是**本 op 的预算）。
///
/// ## 出站形状：嵌一层的 `{"request": {…}}`
///
/// 这是一个 DTO（不是散标量），照 `knowledge.vector` 的 `request` 形态；宿主侧
/// `deserialize_arg(&args, "request", seam)` 只认这一种，平铺会在宿主落
/// `SEAM_CALL_INVALID_ARGS`。多余键不转发（宿主 serde 本来也会忽略未知字段，本层给它同一条口径）。
///
/// ## 出参 = 创建的**列裁剪视图**（逐字）
///
/// 与列表行同一套 10 个 camelCase 键、同样不含正文。带 `id` 回来是关键：调用方拿它做后续
/// 发布 / 删除的句柄（#180-④ 的教训 —— 让前端自己造 id 会落下悬空引用）。
fn create_blog_post(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    let request = blog_post_create_request(input)?;
    call_opc_store(peer, OPC_STORE_CREATE_BLOG_POST, json!({ "request": request }), call_chain)
}

/// 命令层形状闸：从平铺入参里挑出 `CreateBlogPostInput` 的五个字段，重建一个**只含这五个键**
/// 的对象交给接缝。
///
/// 判据分层与 `publish` / `delete` 同一条：这里只判「键在不在、类型对不对」，不判语义
/// （空串 / 超预算都放行到宿主 —— 那是 provider 的闸）。
fn blog_post_create_request(input: &Value) -> Result<Value, (String, String)> {
    for key in ["title", "slug", "excerpt", "content"] {
        if input.get(key).and_then(Value::as_str).is_none() {
            return Err(invalid_shape(format!(
                "opc_create_blog_post 需要平铺的字符串字段 `{key}`\
                 （形状判在命令层，非空 / 字节预算判在宿主 provider）"
            )));
        }
    }
    let tags = match input.get("tags") {
        Some(Value::Array(items)) if items.iter().all(Value::is_string) => {
            Value::Array(items.clone())
        },
        _ => {
            return Err(invalid_shape(
                "opc_create_blog_post 的 `tags` 必须是字符串数组（形状判在命令层）".to_string(),
            ));
        },
    };
    Ok(json!({
        "title": input["title"],
        "slug": input["slug"],
        "excerpt": input["excerpt"],
        "content": input["content"],
        "tags": tags,
    }))
}

/// 命令层形状错误（`PLUGIN_COMMAND_INVALID_ARGS`）：**不发帧**，也就不存在「注定失败的帧」。
fn invalid_shape(message: String) -> (String, String) {
    (error_codes::PLUGIN_COMMAND_INVALID_ARGS.to_string(), message)
}

// ── #36 批 1：只读查询命令 ──────────────────────────────────────────────

/// 四条带过滤条件的命令各自认的入参键（camelCase，与契约 `*ListQuery` 的 serde 键逐字一致）。
/// 只挑这些键转发：多余键不进门（同 [`blog_post_create_request`] 的口径）。`list_customers`
/// 刻意不含 `tags` —— 域服务不消费该过滤位，契约也不收（见 `opc_store.rs` 的 `CustomerListQuery`）。
const QUERY_KEYS_INVOICES: &[&str] = &["status", "customerId", "dateFrom", "dateTo", "limit"];
const QUERY_KEYS_CUSTOMERS: &[&str] = &["status", "customerType", "search", "limit"];
const QUERY_KEYS_PROJECTS: &[&str] = &["status", "customerId", "search", "limit"];
const QUERY_KEYS_KPIS: &[&str] = &["period", "limit"];

/// 只读列表命令的公共出口：把 agent 侧传来的 camelCase 键**原样**装进 `request`，出参逐字
/// 透传接缝回帧（裸数组，camelCase 键，零键名改写 —— 同 [`blog_posts`] 的「出参线格式」）。
///
/// - `keys` 为空 ⇒ 该 op 无入参，直接发空入参（不塞 `request` 空壳）；
/// - 缺失的键**不塞默认值、不塞 null**：契约 DTO 是 `default`，缺键即 `None`，塞 null 会
///   多造一个「调用方显式传了 null」的假信号。
fn list_rows(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
    op: &str,
    keys: &[&str],
) -> Result<Value, (String, String)> {
    if keys.is_empty() {
        return call_opc_store(peer, op, json!({}), call_chain);
    }
    let mut request = serde_json::Map::new();
    for key in keys {
        // 缺键与 null 都不转发（同宿主工具侧 `filter_request` 的口径）。
        if let Some(value) = input.get(*key).filter(|value| !value.is_null()) {
            request.insert((*key).to_string(), value.clone());
        }
    }
    call_opc_store(peer, op, json!({ "request": request }), call_chain)
}

/// 回帧里的行数组（缺键 / 非数组 ⇒ 空数组，**不** panic）：空表 = 空数组不是错误。
fn rows_of(value: &Value) -> &[Value] {
    match value.as_array() {
        Some(rows) => rows,
        None => &[],
    }
}

/// 行对象里某个字符串字段（缺键 / 非字符串 ⇒ `None`，调用方按「不匹配」处理）。
fn row_str<'a>(row: &'a Value, key: &str) -> Option<&'a str> {
    row.get(key).and_then(Value::as_str)
}

/// `opc_get_dashboard`：仪表盘六项读数，由本进程组合三次只读列表查询现算。
///
/// 宿主**不**新增聚合 op（同一条 `opc.store` 数据面只开列表读面）：三张表各取一页
/// `limit: 100` 后按视图字段现算 —— **超出这一页的行不计入聚合**（这是本命令的取值边界，
/// 不是宿主 provider 的缺省）。空表 ⇒ 各项为 0。
fn dashboard(call_chain: &[String], peer: &mut WorkerPeer) -> Result<Value, (String, String)> {
    let invoices = call_opc_store(
        peer,
        OPC_STORE_LIST_INVOICES,
        json!({ "request": { "limit": 100 } }),
        call_chain,
    )?;
    let customers = call_opc_store(
        peer,
        OPC_STORE_LIST_CUSTOMERS,
        json!({ "request": { "limit": 100 } }),
        call_chain,
    )?;
    let projects = call_opc_store(
        peer,
        OPC_STORE_LIST_PROJECTS,
        json!({ "request": { "limit": 100 } }),
        call_chain,
    )?;

    let invoice_rows = rows_of(&invoices);
    let customer_rows = rows_of(&customers);
    let project_rows = rows_of(&projects);

    let total_revenue: f64 = invoice_rows
        .iter()
        .filter(|row| row_str(row, "status") == Some("paid"))
        .filter_map(|row| row.get("total").and_then(Value::as_f64))
        .sum();
    let pending_invoices = invoice_rows
        .iter()
        .filter(|row| matches!(row_str(row, "status"), Some("draft" | "sent" | "overdue")))
        .count();
    let active_projects = project_rows
        .iter()
        .filter(|row| matches!(row_str(row, "status"), Some("active" | "planning")))
        .count();
    let active_customers =
        customer_rows.iter().filter(|row| row_str(row, "status") == Some("active")).count();

    // 键名 camelCase：宿主工具按这套键格式化，改一个字母就会静默读空。
    Ok(json!({
        "totalRevenue": total_revenue,
        "pendingInvoices": pending_invoices,
        "activeProjects": active_projects,
        "activeCustomers": active_customers,
        "totalCustomers": customer_rows.len(),
        "totalProjects": project_rows.len(),
    }))
}

// ── 测试 ────────────────────────────────────────────────────────────────
//
// 内联测试 mod 追加在文件末尾（`items_after_test_module` 只在 clippy 下暴露）。
//
// 只测三件纯函数事：回帧取列的容错、「出口零键名改写 + 键集合逐字 camelCase」的形态锁，
// 以及**创建命令的形状闸**（`blog_post_create_request`：五个字段齐 + 类型对才放行、
// 多余键不转发、语义闸不越界）。「真管道上发帧 / 分页取尽 / 三条写命令的出参两态与三态」
// 归 `tests/protocol.rs`（需要真进程与真 stdin/stdout）。
//
// ⚠ 三条写命令（[`publish_blog_post`] / [`delete_blog_post`] / [`create_blog_post`]）在这里
// **没有**端到端用例，不是漏测而是够不着：它们的签名要 `&mut WorkerPeer`（真管道才构造得出来），
// 入参形状与两态 / 三态透传全部归 `tests/protocol.rs` 的 `command_frame_*` 三条用例钉住 ——
// 那里同时锁出站帧的 seam / op 逐字、`args` 的键集合、以及 `not_found` / `protected`
// 走 payload 不走错误码。

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
