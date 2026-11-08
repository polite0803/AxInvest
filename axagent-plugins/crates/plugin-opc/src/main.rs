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

/// #36 批 2 的四条写 op（同样要 `write`）：发票状态变更 / 内容资产更新 / 内容资产删除 /
/// 发布计划取消。出参一律是契约的外部标记枚举（`{"transitioned": …}` / `{"updated": …}` /
/// `"removed"` / `"not_found"`），本进程逐字透传。
const OPC_STORE_TRANSITION_INVOICE: &str = "transition_invoice";
const OPC_STORE_UPDATE_CONTENT_ASSET: &str = "update_content_asset";
const OPC_STORE_DELETE_CONTENT_ASSET: &str = "delete_content_asset";
const OPC_STORE_CANCEL_PUBLISH_SCHEDULE: &str = "cancel_publish_schedule";

/// #36 批 3 的九条 op：八个写 op（要 `write`）+ 一条读 op（财务报表，宿主侧无权限闸）。
/// 出参一律是契约的列裁剪视图 / 视图数组，逐字透传（同 `create_blog_post` 的判据）。
const OPC_STORE_CREATE_INVOICE: &str = "create_invoice";
const OPC_STORE_CREATE_CUSTOMER: &str = "create_customer";
const OPC_STORE_CREATE_PROJECT: &str = "create_project";
const OPC_STORE_ADD_MILESTONE: &str = "add_milestone";
const OPC_STORE_CREATE_LANDING_PAGE: &str = "create_landing_page";
const OPC_STORE_GET_FINANCIAL_REPORT: &str = "get_financial_report";
const OPC_STORE_CREATE_CONTENT_ASSET: &str = "create_content_asset";
const OPC_STORE_CREATE_PUBLISH_SCHEDULE: &str = "create_publish_schedule";
const OPC_STORE_PROCESS_DUE_SCHEDULES: &str = "process_due_schedules";

/// #32 批 A 的三条 op（都要 `write`）：客户更新 / 客户删除 / 发票删除。前两者同属
/// 客户面，`delete_invoice` 删的是**域包发票**——宿主同名 Tauri 命令删的是交付域发票
/// （两套体系同名不同义，见 `worker.rs` 分派臂的警示）。`update_customer` 出参是
/// [`CustomerSummaryView`] 逐字透传；两条删除 op 成功无载荷（接缝出 `Value::Null`）。
const OPC_STORE_UPDATE_CUSTOMER: &str = "update_customer";
const OPC_STORE_DELETE_CUSTOMER: &str = "delete_customer";
const OPC_STORE_DELETE_INVOICE: &str = "delete_invoice";

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

/// #36 批 2 的四条写命令（清单 `commands[].name` 必须逐字一致）。
const COMMAND_TRANSITION_INVOICE: &str = "opc_transition_invoice";
const COMMAND_UPDATE_CONTENT_ASSET: &str = "opc_update_content_asset";
const COMMAND_DELETE_CONTENT_ASSET: &str = "opc_delete_content_asset";
const COMMAND_CANCEL_PUBLISH_SCHEDULE: &str = "opc_cancel_publish_schedule";

/// #36 批 3 的九条命令（清单 `commands[].name` 必须逐字一致）：八条写命令 + 一条只读的
/// 财务报表（宿主侧无权限闸，与八个 list op 同一档）。
const COMMAND_CREATE_INVOICE: &str = "opc_create_invoice";
const COMMAND_CREATE_CUSTOMER: &str = "opc_create_customer";
const COMMAND_CREATE_PROJECT: &str = "opc_create_project";
const COMMAND_ADD_MILESTONE: &str = "opc_add_milestone";
const COMMAND_CREATE_LANDING_PAGE: &str = "opc_create_landing_page";
const COMMAND_GET_FINANCIAL_REPORT: &str = "opc_get_financial_report";
const COMMAND_CREATE_CONTENT_ASSET: &str = "opc_create_content_asset";
const COMMAND_CREATE_PUBLISH_SCHEDULE: &str = "opc_create_publish_schedule";
const COMMAND_PROCESS_DUE_SCHEDULES: &str = "opc_process_due_schedules";

/// #32 批 A 的三条写命令（清单 `commands[].name` 必须逐字一致）。`opc_update_customer` /
/// `opc_delete_customer` 与宿主同名 Tauri 命令同义（批 C 删宿主侧后本条是唯一通道）；
/// `opc_delete_invoice` 与宿主同名 Tauri 命令**同名不同义**（域包发票 vs 交付域发票）。
const COMMAND_UPDATE_CUSTOMER: &str = "opc_update_customer";
const COMMAND_DELETE_CUSTOMER: &str = "opc_delete_customer";
const COMMAND_DELETE_INVOICE: &str = "opc_delete_invoice";

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
        // ── #36 批 2：四条写命令（出参 = 接缝外部标记枚举，逐字透传）──
        COMMAND_TRANSITION_INVOICE => {
            match transition_invoice(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_UPDATE_CONTENT_ASSET => {
            match update_content_asset(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_DELETE_CONTENT_ASSET => {
            match delete_content_asset(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_CANCEL_PUBLISH_SCHEDULE => {
            match cancel_publish_schedule(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // ── #36 批 3：八条写命令 + 一条只读报表（出参 = 契约列裁剪视图，逐字透传）──
        COMMAND_CREATE_INVOICE => match create_invoice(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_CREATE_CUSTOMER => match create_customer(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // ── #32 批 A：三条写命令（更新出参 = 列裁剪视图，删除出参 = 无载荷）──
        COMMAND_UPDATE_CUSTOMER => match update_customer(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_DELETE_CUSTOMER => match delete_customer(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_DELETE_INVOICE => match delete_invoice(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_CREATE_PROJECT => match create_project(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_ADD_MILESTONE => match add_milestone(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_CREATE_LANDING_PAGE => {
            match create_landing_page(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_GET_FINANCIAL_REPORT => {
            match get_financial_report(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_CREATE_CONTENT_ASSET => {
            match create_content_asset(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_CREATE_PUBLISH_SCHEDULE => {
            match create_publish_schedule(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_PROCESS_DUE_SCHEDULES => match process_due_schedules(&request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
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

// ── #36 批 2：四条写命令 ────────────────────────────────────────────────

/// `opc_transition_invoice`：变更发票状态（写 op）。
///
/// ## 入参只判形状
///
/// `id` / `status` 都必须在且是字符串（不是 ⇒ `PLUGIN_COMMAND_INVALID_ARGS`，**不发帧**）。
/// 语义闸全在宿主 provider：`status` 的枚举合法性（非法串 ⇒ `SEAM_CALL_INVALID_ARGS`）与
/// 状态转换合法性（非法转换 ⇒ `PLUGIN_CALL_FAILED`）。
///
/// ## 出站形状：`{"id", "target"}`
///
/// 契约 op 的第二个键叫 `target`（与域服务 `transition_status(id, target)` 同名）；命令面
/// 对外用 `status`（与清单 inputSchema / 智能体工具入参一致）⇒ 改名只发生在**本函数**。
///
/// ## 出参 = `{"transitioned": {…}}` / `"not_found"`，逐字透传
fn transition_invoice(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    let Some(id) = input.get("id").and_then(Value::as_str) else {
        return Err(invalid_shape(
            "opc_transition_invoice 需要字符串 id（形状判在命令层）".to_string(),
        ));
    };
    let Some(status) = input.get("status").and_then(Value::as_str) else {
        return Err(invalid_shape(
            "opc_transition_invoice 需要字符串 status（合法性判在宿主 provider）".to_string(),
        ));
    };
    call_opc_store(
        peer,
        OPC_STORE_TRANSITION_INVOICE,
        json!({ "id": id, "target": status }),
        call_chain,
    )
}

/// `opc_update_content_asset`：更新内容资产（写 op）。
///
/// 入参是平铺的 `id` + 五个可选字段；出站是 `{"id", "request": {…}}`（`request` 是一个
/// DTO，必须嵌层 —— 与 `create_blog_post` 同一条判据）。缺键 = 该列不动。
fn update_content_asset(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    let Some(id) = input.get("id").and_then(Value::as_str) else {
        return Err(invalid_shape(
            "opc_update_content_asset 需要字符串 id（形状判在命令层）".to_string(),
        ));
    };
    let request = content_asset_update_request(input)?;
    call_opc_store(
        peer,
        OPC_STORE_UPDATE_CONTENT_ASSET,
        json!({ "id": id, "request": request }),
        call_chain,
    )
}

/// 命令层形状闸：从平铺入参里挑出 `UpdateContentAssetInput` 的五个字段。
///
/// 判据同 [`blog_post_create_request`]：只判「键在不在、类型对不对」，不判语义（空串 /
/// 非法状态串都放行到宿主 —— 域服务与契约各自收口）。`contentType` 是 camelCase 线格式
/// （禁区 13），本层不做 snake_case 兼容翻译。
fn content_asset_update_request(input: &Value) -> Result<Value, (String, String)> {
    let mut request = serde_json::Map::new();
    for key in ["title", "contentType", "body", "status"] {
        match input.get(key) {
            None | Some(Value::Null) => {},
            Some(Value::String(value)) => {
                request.insert(key.to_string(), Value::String(value.clone()));
            },
            Some(_) => {
                return Err(invalid_shape(format!(
                    "opc_update_content_asset 的 `{key}` 必须是字符串（形状判在命令层）"
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
                "opc_update_content_asset 的 `tags` 必须是字符串数组（形状判在命令层）".to_string(),
            ));
        },
    }
    Ok(Value::Object(request))
}

/// `opc_delete_content_asset`：删除内容资产（写 op，两态出参逐字透传）。
fn delete_content_asset(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    let Some(id) = input.get("id").and_then(Value::as_str) else {
        return Err(invalid_shape(
            "opc_delete_content_asset 需要字符串 id（形状判在命令层）".to_string(),
        ));
    };
    call_opc_store(peer, OPC_STORE_DELETE_CONTENT_ASSET, json!({ "id": id }), call_chain)
}

/// `opc_cancel_publish_schedule`：取消发布计划（写 op，**硬删**；两态出参逐字透传）。
fn cancel_publish_schedule(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    let Some(id) = input.get("id").and_then(Value::as_str) else {
        return Err(invalid_shape(
            "opc_cancel_publish_schedule 需要字符串 id（形状判在命令层）".to_string(),
        ));
    };
    call_opc_store(peer, OPC_STORE_CANCEL_PUBLISH_SCHEDULE, json!({ "id": id }), call_chain)
}

// ── #36 批 3：八条写命令 + 一条只读报表 ────────────────────────────────
//
// 判据与批 2 同一条：**命令层只判形状**（缺必需键 / 类型不对 ⇒ `PLUGIN_COMMAND_INVALID_ARGS`，
// **不发帧**），语义与闸全在宿主 provider（空串 / 非法枚举 / 缺行由它各自收口）。出参是契约
// 的列裁剪视图（或视图数组），**逐字透传** —— 本层不做任何键名改写或业务判断。
//
// 入参键一律 camelCase（禁区 13，与契约 DTO 的 serde 线格式同源）。数组字段（`lineItems` /
// `tags`）只判「是不是对象数组 / 字符串数组」，元素内部字段类型的错由宿主 serde 拒收
// （`SEAM_CALL_INVALID_ARGS`）—— 逐元素逐字段再判一遍只会把同一件事判两次。

/// `opc_create_invoice`：创建发票（写 op）。
///
/// 行项目的 `total` **不在这里算**：由宿主 provider 按 `quantity × unitPrice × (1 + taxRate)`
/// 现算（迁移前工具在本进程算，上收后单价/税率的缺省口径只有一个落点）。
fn create_invoice(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_create_invoice";
    let customer_id = require_str(input, OP, "customerId")?;
    let line_items = object_array(input, OP, "lineItems")?;
    let mut request = serde_json::Map::new();
    request.insert("customerId".to_string(), Value::String(customer_id.to_string()));
    request.insert("lineItems".to_string(), line_items);
    copy_optional_strings(input, OP, &mut request, &["currency", "notes"])?;
    copy_optional_i64(input, OP, &mut request, &["dueAt"])?;
    call_opc_store(
        peer,
        OPC_STORE_CREATE_INVOICE,
        json!({ "request": Value::Object(request) }),
        call_chain,
    )
}

/// `opc_create_customer`：创建客户（写 op）。
///
/// `customerType` / `source` 收**串**：未知值分别由宿主落 `Unknown` / `Other(原串)`，
/// 不在此处把枚举抄第二遍（迁移前工具的映射原样下沉到 provider）。
fn create_customer(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_create_customer";
    let name = require_str(input, OP, "name")?;
    let email = require_str(input, OP, "email")?;
    let mut request = serde_json::Map::new();
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
    call_opc_store(
        peer,
        OPC_STORE_CREATE_CUSTOMER,
        json!({ "request": Value::Object(request) }),
        call_chain,
    )
}

/// `opc_update_customer`：更新客户（#32 批 A 的写 op）。
///
/// `id` 必填（散标量）；正文平铺可选字段组装 `{"request": {…}}`。可选字段缺键 /
/// `null` 都不转发（helper 口径）⇒ 接缝 DTO 外层 `None` = 不动；给了值 = 写入。与
/// 接缝注释一致：**清空（域包 `Some(None)`）在本命令的 JSON 线格式上不可表达**，与
/// 既存宿主 Tauri 命令行为完全一致。出参 = 更新后的 [`CustomerSummaryView`]。
fn update_customer(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_update_customer";
    let id = require_str(input, OP, "id")?;
    let mut request = serde_json::Map::new();
    copy_optional_strings(
        input,
        OP,
        &mut request,
        &[
            "name",
            "email",
            "phone",
            "company",
            "customerType",
            "country",
            "region",
            "city",
            "address",
            "notes",
        ],
    )?;
    copy_optional_f64(input, OP, &mut request, &["latitude", "longitude"])?;
    call_opc_store(
        peer,
        OPC_STORE_UPDATE_CUSTOMER,
        json!({ "id": id, "request": Value::Object(request) }),
        call_chain,
    )
}

/// `opc_delete_customer`：删除一个客户（#32 批 A 的写 op）。
///
/// 入参只有 `id`，且只判形状（非空判在宿主 provider，同 [`publish_blog_post`] 的分工）。
/// 成功无载荷（接缝出 `Value::Null`，逐字透传）。
fn delete_customer(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_delete_customer";
    let id = require_str(input, OP, "id")?;
    call_opc_store(peer, OPC_STORE_DELETE_CUSTOMER, json!({ "id": id }), call_chain)
}

/// `opc_delete_invoice`：删除一张**域包发票**（#32 批 A 的写 op）。
///
/// ⚠ 与宿主同名 Tauri 命令**同名不同义**：宿主命令删的是交付域发票
/// （`opc_delivery` repo），本命令删的是 `opc_invoices` 表的域包发票 ——
/// 前端发票页两套体系并存时靠命令通道区分。入参只有 `id`（只判形状）。
fn delete_invoice(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_delete_invoice";
    let id = require_str(input, OP, "id")?;
    call_opc_store(peer, OPC_STORE_DELETE_INVOICE, json!({ "id": id }), call_chain)
}

/// `opc_create_project`：创建项目（写 op）。
fn create_project(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_create_project";
    let title = require_str(input, OP, "title")?;
    let mut request = serde_json::Map::new();
    request.insert("title".to_string(), Value::String(title.to_string()));
    copy_optional_strings(
        input,
        OP,
        &mut request,
        &["customerId", "description", "currency", "notes"],
    )?;
    copy_optional_f64(input, OP, &mut request, &["budget"])?;
    copy_optional_i64(input, OP, &mut request, &["deadline"])?;
    call_opc_store(
        peer,
        OPC_STORE_CREATE_PROJECT,
        json!({ "request": Value::Object(request) }),
        call_chain,
    )
}

/// `opc_add_milestone`：给项目加里程碑（写 op）。
///
/// 里程碑 id 与初始状态由宿主 provider 生成（`Pending`）—— 命令面不收 `id` / `status`。
fn add_milestone(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_add_milestone";
    let project_id = require_str(input, OP, "projectId")?;
    let title = require_str(input, OP, "title")?;
    let mut request = serde_json::Map::new();
    request.insert("projectId".to_string(), Value::String(project_id.to_string()));
    request.insert("title".to_string(), Value::String(title.to_string()));
    copy_optional_strings(input, OP, &mut request, &["description"])?;
    copy_optional_i64(input, OP, &mut request, &["dueAt"])?;
    call_opc_store(
        peer,
        OPC_STORE_ADD_MILESTONE,
        json!({ "request": Value::Object(request) }),
        call_chain,
    )
}

/// `opc_create_landing_page`：创建落地页（写 op，创建后为草稿态）。
fn create_landing_page(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_create_landing_page";
    let title = require_str(input, OP, "title")?;
    let slug = require_str(input, OP, "slug")?;
    let content = require_str(input, OP, "content")?;
    let mut request = serde_json::Map::new();
    request.insert("title".to_string(), Value::String(title.to_string()));
    request.insert("slug".to_string(), Value::String(slug.to_string()));
    request.insert("content".to_string(), Value::String(content.to_string()));
    copy_optional_strings(input, OP, &mut request, &["description"])?;
    call_opc_store(
        peer,
        OPC_STORE_CREATE_LANDING_PAGE,
        json!({ "request": Value::Object(request) }),
        call_chain,
    )
}

/// `opc_get_financial_report`：取财务报表（只读 op，宿主侧无写权限闸）。
///
/// 入参是**散标量** `{"period": …}`（不是 DTO，不套 `request` 壳）；出参是
/// [`FinancialReportView`] 的扁平对象 —— 报表与投资建议成对返回，只多一次帧往返。
fn get_financial_report(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_get_financial_report";
    let period = require_str(input, OP, "period")?;
    call_opc_store(peer, OPC_STORE_GET_FINANCIAL_REPORT, json!({ "period": period }), call_chain)
}

/// `opc_create_content_asset`：创建内容资产（写 op，创建后为草稿态）。
fn create_content_asset(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    const OP: &str = "opc_create_content_asset";
    let title = require_str(input, OP, "title")?;
    let content_type = require_str(input, OP, "contentType")?;
    let body = require_str(input, OP, "body")?;
    let mut request = serde_json::Map::new();
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
///
/// `contentRefId` 是**内容实体的主键**（博客 id / 内容资产 id），不是发布计划自己的 id。
fn create_publish_schedule(
    input: &Value,
    call_chain: &[String],
    peer: &mut WorkerPeer,
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

/// `opc_process_due_schedules`：处理全部到期的发布计划（写 op，无入参）。
///
/// 出参是**本次处理过的行**（视图数组）：空数组 = 「没有到期计划」，不是错误。
fn process_due_schedules(
    call_chain: &[String],
    peer: &mut WorkerPeer,
) -> Result<Value, (String, String)> {
    call_opc_store(peer, OPC_STORE_PROCESS_DUE_SCHEDULES, json!({}), call_chain)
}

/// 必需字符串字段。
fn require_str<'a>(input: &'a Value, op: &str, key: &str) -> Result<&'a str, (String, String)> {
    input
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_shape(format!("{op} 需要字符串字段 `{key}`（形状判在命令层）")))
}

/// 可选字符串字段：缺键 / `null` 不转发；给了非字符串 ⇒ 拒收。
fn copy_optional_strings(
    input: &Value,
    op: &str,
    request: &mut serde_json::Map<String, Value>,
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

/// 可选浮点字段（JSON 只有一种 number 类型，整数也算命中）。
fn copy_optional_f64(
    input: &Value,
    op: &str,
    request: &mut serde_json::Map<String, Value>,
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

/// 可选时间戳字段（整数；秒级 epoch，与库里的时间列同量纲）。
fn copy_optional_i64(
    input: &Value,
    op: &str,
    request: &mut serde_json::Map<String, Value>,
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

/// 必需的对象数组字段（元素内部字段类型由宿主 serde 判）。
fn object_array(input: &Value, op: &str, key: &str) -> Result<Value, (String, String)> {
    let items = input
        .get(key)
        .and_then(Value::as_array)
        .filter(|items| items.iter().all(Value::is_object))
        .ok_or_else(|| invalid_shape(format!("{op} 需要对象数组字段 `{key}`（形状判在命令层）")))?;
    Ok(Value::Array(items.clone()))
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
