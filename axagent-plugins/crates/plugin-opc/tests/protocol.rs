// SPDX-License-Identifier: AGPL-3.0-only
//! 真进程 + 真管道的协议测试：宿主将来就是按这个形态 spawn 它的。
//!
//! 为什么不用进程内替代：`stdin`/`stdout` 的缓冲与 flush、以及「等响应期间对端又发来
//! 请求」的重入语义，只有在真管上才暴露。本文件锁两件事：#36 第 1 批的**骨架契约**
//! （身份、未知命令显式失败、停机正常退出）与第 3–4 批的**第一条出站接缝链路**
//! （`opc_list_blog_posts` → `opc.store::list_blog_posts`，含整表分页取尽）。

use std::io::{BufReader, BufWriter};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use axagent_plugin_proto::{
    AXAGENT_PLUGIN_PROTO_VERSION, CommandFrame, FrameRequest, FrameResponse, PluginDeclaration,
    ResponseKind, error_codes, ops, read_inbound, write_json_frame,
};

/// 宿主 spawn 内置包时注入的**真实**身份形态：`{小写 name}@{marketplace}`
/// （`plugins/src/manager.rs` 的 `plugin_id()`，`BUNDLED_MARKETPLACE = "bundled"`）。
///
/// ⚠ 这里**不许**与 worker 里的常量同源自证 —— 曾经写成 `"plugin.opc"`，而宿主实际注入
/// `opc@bundled`，两道自检一起错 ⇒ 每一条真实命令都被拒成 `PLUGIN_COMMAND_ID_MISMATCH`，
/// 而测试照样绿（它注入的就是那个错值）。形态须以宿主侧源码为准，另有两条用例分别锁
/// 「外部安装形态放行」与「错配到别的插件被拒」。
const PLUGIN_ID: &str = "opc@bundled";

struct Worker {
    child: Child,
    /// `Option` 只为一件事：停机前要真的关掉写端（EOF 是 worker 的停机信号）。
    stdin: Option<BufWriter<ChildStdin>>,
    stdout: BufReader<ChildStdout>,
}

impl Worker {
    fn spawn() -> Self {
        Self::spawn_as(PLUGIN_ID)
    }

    /// 以指定身份注入启动 —— 两条身份用例要靠它，否则测试只能自证。
    fn spawn_as(injected_id: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_axagent-plugin-opc"))
            // 宿主 spawn 时会注入自身身份；命令帧的身份自检依赖它（缺省即空串 ⇒ 任何帧都拒）。
            .env("AXAGENT_PLUGIN_ID", injected_id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("worker 二进制应能启动");
        let stdin = BufWriter::new(child.stdin.take().expect("stdin 已被设为 piped"));
        let stdout = BufReader::new(child.stdout.take().expect("stdout 已被设为 piped"));
        Self { child, stdin: Some(stdin), stdout }
    }

    fn send(&mut self, request: &FrameRequest) {
        write_json_frame(self.stdin.as_mut().expect("写端仍打开"), request).expect("写帧应成功");
    }

    fn send_response(&mut self, response: &FrameResponse) {
        write_json_frame(self.stdin.as_mut().expect("写端仍打开"), response).expect("写帧应成功");
    }

    /// 读一帧**请求**（worker → 宿主，出站 `call_seam` 走的就是这一向）。
    fn recv_request(&mut self) -> FrameRequest {
        match read_inbound(&mut self.stdout).expect("读帧应成功").as_request().cloned() {
            Some(request) => request,
            None => panic!("期望一帧请求（worker → 宿主），实际是响应"),
        }
    }

    fn recv_response(&mut self) -> FrameResponse {
        match read_inbound(&mut self.stdout).expect("读帧应成功").as_response().cloned() {
            Some(response) => response,
            None => panic!("期望一帧响应（宿主 → worker 的请求被回执），实际是请求"),
        }
    }

    fn close_stdin(&mut self) {
        drop(self.stdin.take());
    }
}

/// 接缝回帧里的一行（`BlogPostView` 的 camelCase 线格式，契约见 harness `src/opc_store.rs`）。
///
/// 刻意**不带** `content`：接缝在 SQL 侧就没 SELECT 正文列，这里也不给它造出该键的机会。
fn seam_row(id: &str, view_count: u32) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "title": format!("第 {id} 篇"),
        "slug": id,
        "excerpt": "摘要",
        "tags": ["opc"],
        "published": true,
        "publishedAt": 1_700_000_000_000_i64,
        "viewCount": view_count,
        "createdAt": 1_700_000_000_000_i64,
        "updatedAt": 1_700_000_060_000_i64,
    })
}

#[test]
fn describe_declares_the_opc_store_outbound_call() {
    let mut worker = Worker::spawn();

    // ① 握手：声明必须合法，且 `calls` 逐条给全 —— 出站准入按它校验，少一条 = 运行时被拒。
    worker.send(
        &FrameRequest::new(
            ops::DESCRIBE,
            serde_json::json!({ "proto_version": AXAGENT_PLUGIN_PROTO_VERSION }),
        )
        .with_id("req-describe"),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Success);
    assert_eq!(response.request_id.as_deref(), Some("req-describe"), "request_id 必须原样回填");
    let declaration: PluginDeclaration =
        serde_json::from_value(response.value.expect("成功帧必须带 value"))
            .expect("声明应是合法 JSON");
    assert_eq!(declaration.proto_version, AXAGENT_PLUGIN_PROTO_VERSION);
    declaration.validate().expect("握手声明必须通过宿主同一套校验");
    assert!(declaration.subscribe.is_empty(), "本插件不订阅任何事件：{:?}", declaration.subscribe);
    assert_eq!(
        declaration.calls,
        vec!["opc.store".to_string()],
        "出站接缝必须逐条声明，否则宿主 handle_call_seam 直接拒调：{:?}",
        declaration.calls
    );
    assert!(
        declaration.capabilities.is_empty(),
        "本插件不向宿主提供任何接缝（opc.store 方向相反）：{:?}",
        declaration.capabilities
    );

    // ② 表外命令名必须显式报未知，且回执回填本插件的身份。
    worker.send(
        &CommandFrame::new(PLUGIN_ID, "opc_not_implemented", serde_json::json!({}), 5_000)
            .to_frame_request()
            .expect("命令帧应能编成请求")
            .with_id("req-command"),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Error);
    assert_eq!(response.request_id.as_deref(), Some("req-command"));
    assert_eq!(
        response.code.as_deref(),
        Some(error_codes::PLUGIN_COMMAND_UNKNOWN),
        "命令表里没有这个名字必须回未知命令，不得静默成功"
    );
    let binding = response.value.expect("错误帧必须透传绑定字段");
    assert_eq!(binding["plugin_id"], PLUGIN_ID, "回执必须回填宿主绑定的身份");
    assert_eq!(binding["command"], "opc_not_implemented");

    // ③ 身份不符：帧绑定别的插件 ⇒ 拒执行（与未知命令是两个错误码），且**不发接缝帧**。
    worker.send(
        &CommandFrame::new("plugin.other", "opc_list_blog_posts", serde_json::json!({}), 5_000)
            .to_frame_request()
            .expect("命令帧应能编成请求"),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Error);
    assert_eq!(
        response.code.as_deref(),
        Some(error_codes::PLUGIN_COMMAND_ID_MISMATCH),
        "帧绑定他人身份必须拒执行"
    );

    // ④ 停机：shutdown 回执后关写端，worker 以 0 退出（宿主停机不走 kill）。
    worker.send(&FrameRequest::new(ops::SHUTDOWN, serde_json::json!({})));
    assert_eq!(worker.recv_response().kind, ResponseKind::Success);
    worker.close_stdin();
    let status = worker.child.wait().expect("等待 worker 退出");
    assert!(status.success(), "收到 shutdown 并关管道后应以 0 退出，实际 {status:?}");
}

/// 身份自检的**第二道**（二进制归属）只在宿主侧源码给出的 id 形态下才成立 —— 两条用例分别锁
/// 「形态不许写死成一个」与「错配到别的插件必须拒」。
///
/// 为什么必须单独测：第一道自检比的是「帧绑定值 = 注入值」，宿主若把 OPC 的帧错配到别的
/// worker，这两个值是**一起错**的 ⇒ 第一道完全看不出来；而第二道若写成「注入值 = 某个写死的
/// 完整 id」，就会在 `@external` 形态下把自家插件也拒掉（本仓踩过：worker 写死 `plugin.opc`，
/// 宿主注入 `opc@bundled`，每条真实命令恒失败，而测试同源注入同一个错值所以照样绿）。
#[test]
fn identity_check_accepts_any_marketplace_but_rejects_foreign_plugin() {
    // 正控：外部安装形态（`{name}@external`）身份必须过 —— 用未知命令名做探针，
    // 过与不过的错误码是两个，能机械区分。
    let mut external = Worker::spawn_as("opc@external");
    external.send(
        &CommandFrame::new("opc@external", "opc_not_implemented", serde_json::json!({}), 5_000)
            .to_frame_request()
            .expect("命令帧应能编成请求"),
    );
    let response = external.recv_response();
    assert_eq!(response.kind, ResponseKind::Error);
    assert_eq!(
        response.code.as_deref(),
        Some(error_codes::PLUGIN_COMMAND_UNKNOWN),
        "`opc@external` 属本插件 ⇒ 不该落 ID_MISMATCH，实际 {:?}",
        response.code
    );

    // 负控：注入值与帧绑定值**一致地**指向别的插件 ⇒ 第一道放行、第二道必须拒。
    let mut misrouted = Worker::spawn_as("stock@bundled");
    misrouted.send(
        &CommandFrame::new("stock@bundled", "opc_not_implemented", serde_json::json!({}), 5_000)
            .to_frame_request()
            .expect("命令帧应能编成请求"),
    );
    let response = misrouted.recv_response();
    assert_eq!(
        response.code.as_deref(),
        Some(error_codes::PLUGIN_COMMAND_ID_MISMATCH),
        "身份一致但属于别的插件 ⇒ 必须按归属拒，不许落进命令表：{:?}",
        response.code
    );
}

/// `opc_list_blog_posts` 的帧级实证（#36 第 3–4 步：OPC 第一条宿主查询接缝）。
///
/// 锁四件「只有真管道能钉住」的跨层口径：
/// 1. 出站帧的 seam / op 逐字是 `opc.store` / `list_blog_posts`（写错 ⇒ 宿主取不到 provider）；
/// 2. 调用方没给分页 ⇒ 缺省只带 `offset: 0`、**不带 `limit`**（缺省档归宿主，插件不另抄数字）；
/// 3. 整表语义：`total` 大于本页 ⇒ 必须再发一帧（`offset` 按**实际取回条数**推进），
///    直到取满才回执 —— 前端 `blog_posts_count` 取的就是这个数组的 `length`；
/// 4. 出参线格式是 snake_case（既存前端的 `interface BlogPost` 与被替代的宿主 DTO 同形），
///    且 `content` 不出现。
#[test]
fn command_frame_lists_opc_blog_posts_via_host_seam() {
    let mut worker = Worker::spawn();
    worker.send(
        &CommandFrame::new(PLUGIN_ID, "opc_list_blog_posts", serde_json::json!({}), 5_000)
            .to_frame_request()
            .expect("命令帧应能编成请求")
            .with_id("req-posts"),
    );

    // 第一页：宿主回 1 条、`total = 2` ⇒ worker 必须继续翻页，而不是就地回执。
    let first = worker.recv_request();
    assert_eq!(first.op, ops::CALL_SEAM);
    assert_eq!(first.args["seam"], "opc.store");
    assert_eq!(first.args["op"], "list_blog_posts");
    assert_eq!(first.args["args"]["offset"], 0);
    assert!(
        first.args["args"].get("limit").is_none(),
        "未指定 limit ⇒ 交给宿主取契约缺省档，插件不抄第二个数字：{:?}",
        first.args
    );
    worker.send_response(
        &FrameResponse::success(
            serde_json::json!({ "posts": [seam_row("post-1", 42)], "total": 2 }),
        )
        .with_id(first.request_id.clone().expect("出站请求必须带 request_id")),
    );

    let second = worker.recv_request();
    assert_eq!(second.args["seam"], "opc.store");
    assert_eq!(
        second.args["args"]["offset"], 1,
        "offset 按实际取回的条数推进（不是按请求的 limit）：{:?}",
        second.args
    );
    worker.send_response(
        &FrameResponse::success(
            serde_json::json!({ "posts": [seam_row("post-2", 7)], "total": 2 }),
        )
        .with_id(second.request_id.clone().expect("出站请求必须带 request_id")),
    );

    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Success, "取尽后应成功回执：{response:?}");
    assert_eq!(response.request_id.as_deref(), Some("req-posts"));
    let reply = response.value.expect("命令回执必须带 value");
    assert_eq!(reply["plugin_id"], PLUGIN_ID);
    let rows = reply["value"].as_array().expect("列表命令必须回数组").clone();
    assert_eq!(rows.len(), 2, "total=2 必须取尽成 2 行：{rows:?}");
    assert_eq!(rows[0]["id"], "post-1");
    assert_eq!(rows[0]["view_count"], 42, "viewCount → view_count 是给前端表格列的线格式");
    assert_eq!(rows[0]["published_at"], 1_700_000_000_000_i64);
    assert_eq!(rows[1]["id"], "post-2");
    // 线格式形态锁：camelCase 源键不透出，正文列不出现。
    for bad in ["viewCount", "publishedAt", "createdAt", "updatedAt", "content"] {
        assert!(rows[0].get(bad).is_none(), "不得透出接缝侧键 {bad}：{}", rows[0]);
    }

    worker.send(&FrameRequest::new(ops::SHUTDOWN, serde_json::json!({})));
    assert_eq!(worker.recv_response().kind, ResponseKind::Success);
    worker.close_stdin();
    assert!(worker.child.wait().expect("等待 worker 退出").success());
}

/// 显式分页：`limit` / `offset` 原样透传、**只取一页**、不按 `total` 续翻。
///
/// 与上一条用例合起来才锁住判据：给了分页就是「我要这一页」，插件替它凑整表会把
/// 调用方的分页算术改写掉。
#[test]
fn explicit_pagination_is_forwarded_verbatim_and_stops_at_one_page() {
    let mut worker = Worker::spawn();
    worker.send(
        &CommandFrame::new(
            PLUGIN_ID,
            "opc_list_blog_posts",
            serde_json::json!({ "limit": 20, "offset": 40 }),
            5_000,
        )
        .to_frame_request()
        .expect("命令帧应能编成请求")
        .with_id("req-page"),
    );
    let call = worker.recv_request();
    assert_eq!(call.op, ops::CALL_SEAM);
    assert_eq!(call.args["seam"], "opc.store");
    assert_eq!(call.args["args"]["limit"], 20);
    assert_eq!(call.args["args"]["offset"], 40, "分页参数必须原样透传：{:?}", call.args);

    // total 远大于本页条数：显式分页下**不该**再发第二帧，直接回执这一页。
    worker.send_response(
        &FrameResponse::success(
            serde_json::json!({ "posts": [seam_row("post-9", 1)], "total": 500 }),
        )
        .with_id(call.request_id.clone().expect("出站请求必须带 request_id")),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Success);
    let rows = response.value.expect("命令回执必须带 value")["value"]
        .as_array()
        .expect("应回数组")
        .clone();
    assert_eq!(rows.len(), 1, "显式分页只交一页：{rows:?}");

    worker.send(&FrameRequest::new(ops::SHUTDOWN, serde_json::json!({})));
    assert_eq!(worker.recv_response().kind, ResponseKind::Success);
    worker.close_stdin();
    assert!(worker.child.wait().expect("等待 worker 退出").success());
}

/// 接缝报错 ⇒ 命令失败并**原样透传错误码**，绝不退化成空数组。
///
/// 空数组在页面上与「一篇都没有」长得一样 —— 静默降级就是把故障伪装成空数据。
#[test]
fn seam_error_is_propagated_as_command_failure() {
    let mut worker = Worker::spawn();
    worker.send(
        &CommandFrame::new(PLUGIN_ID, "opc_list_blog_posts", serde_json::json!({}), 5_000)
            .to_frame_request()
            .expect("命令帧应能编成请求")
            .with_id("req-error"),
    );
    let call = worker.recv_request();
    assert_eq!(call.op, ops::CALL_SEAM);
    worker.send_response(
        &FrameResponse::error("TEST_OPC_STORE_UNAVAILABLE", "宿主 provider 未注册")
            .with_id(call.request_id.clone().expect("出站请求必须带 request_id")),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Error, "接缝失败必须让命令失败");
    assert_eq!(response.code.as_deref(), Some("TEST_OPC_STORE_UNAVAILABLE"), "错误码原样透传");
    assert_eq!(response.request_id.as_deref(), Some("req-error"));

    worker.send(&FrameRequest::new(ops::SHUTDOWN, serde_json::json!({})));
    assert_eq!(worker.recv_response().kind, ResponseKind::Success);
    worker.close_stdin();
    assert!(worker.child.wait().expect("等待 worker 退出").success());
}
