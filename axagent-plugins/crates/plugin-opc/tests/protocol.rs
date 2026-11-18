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
/// 4. 出参线格式 = 接缝视图逐字 camelCase（禁区 13：插件出口不得有第二套键名，前端
///    `SitesTab.tsx` 的 `interface BlogPost` 与表格列 `dataIndex` 就是这套键名），
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
    assert_eq!(rows[0]["viewCount"], 42, "接缝的 camelCase 键必须原样透出（禁区 13）");
    assert_eq!(rows[0]["publishedAt"], 1_700_000_000_000_i64);
    assert_eq!(rows[1]["id"], "post-2");
    // 线格式形态锁：键集合逐字 = 接缝视图的 10 个 camelCase 键，出口不得改写。
    let mut keys: Vec<String> = rows[0].as_object().expect("行应是对象").keys().cloned().collect();
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
    for bad in ["view_count", "published_at", "created_at", "updated_at", "content"] {
        assert!(
            rows[0].get(bad).is_none(),
            "不得出现 snake_case 改写键 / 正文列键 {bad}：{}",
            rows[0]
        );
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

/// `opc_publish_blog_post` 的帧级实证（本批：OPC **第一条写面**命令）。
///
/// 锁四件只有真管道能钉住的跨层口径：
/// 1. 出站帧的 seam / op 逐字 = `opc.store` / `publish_blog_post` —— op 名写错会落
///    `SEAM_CALL_UNSUPPORTED`，前端只看得见「发布失败」，归因成本极高；
/// 2. 出站 `args` **只有 `id` 一个键**、值与命令入参逐字相同（不带 `content`、不套
///    `request` 外层 —— 嵌套形态是 `knowledge.vector` 那类 DTO op 的约定，这里没有 DTO）；
/// 3. 两态都走 payload：`"published"` 与 `"not_found"` 都是 `ResponseKind::Success`，
///    值**原样**成为命令出参（不包装、不改写、不补默认）；
/// 4. 宿主侧真失败 ⇒ 原样透传错误码，不退化成「成功但没发布」。
#[test]
fn command_frame_publishes_a_blog_post_via_host_seam() {
    let mut worker = Worker::spawn();

    // ① + ③ 命中态。
    worker.send(
        &CommandFrame::new(
            PLUGIN_ID,
            "opc_publish_blog_post",
            serde_json::json!({ "id": "post-7" }),
            5_000,
        )
        .to_frame_request()
        .expect("命令帧应能编成请求")
        .with_id("req-publish"),
    );
    let call = worker.recv_request();
    assert_eq!(call.op, ops::CALL_SEAM, "写面同样经 call_seam 出站");
    assert_eq!(call.args["seam"], "opc.store");
    assert_eq!(call.args["op"], "publish_blog_post", "op 名必须逐字");
    let args = call.args["args"].as_object().expect("出站 args 应是对象");
    assert_eq!(
        args.keys().cloned().collect::<Vec<String>>(),
        vec!["id".to_string()],
        "写面入参只有 id 一个键（不许顺手带上正文或其它字段）：{args:?}"
    );
    assert_eq!(args["id"], "post-7", "id 必须逐字透传，插件侧不做任何加工");
    worker.send_response(
        &FrameResponse::success(serde_json::json!("published"))
            .with_id(call.request_id.clone().expect("出站请求必须带 request_id")),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Success, "命中应回成功：{response:?}");
    assert_eq!(response.request_id.as_deref(), Some("req-publish"));
    let reply = response.value.expect("命令回执必须带 value");
    assert_eq!(reply["plugin_id"], PLUGIN_ID);
    assert_eq!(reply["value"], serde_json::json!("published"), "两态串逐字透出，不建包装对象");

    // ② 缺行态：仍然 Success，值 `"not_found"` —— **不许**借错误码表达。
    worker.send(
        &CommandFrame::new(
            PLUGIN_ID,
            "opc_publish_blog_post",
            serde_json::json!({ "id": "gone" }),
            5_000,
        )
        .to_frame_request()
        .expect("命令帧应能编成请求")
        .with_id("req-absent"),
    );
    let call = worker.recv_request();
    assert_eq!(call.args["op"], "publish_blog_post");
    worker.send_response(
        &FrameResponse::success(serde_json::json!("not_found"))
            .with_id(call.request_id.clone().expect("出站请求必须带 request_id")),
    );
    let response = worker.recv_response();
    assert_eq!(
        response.kind,
        ResponseKind::Success,
        "「这一篇不在表里」是正常结果，落成错误就等于让前端把两件事读成一件事：{response:?}"
    );
    assert_eq!(response.code, None, "成功帧不该带错误码：{response:?}");
    assert_eq!(
        response.value.expect("命令回执必须带 value")["value"],
        serde_json::json!("not_found")
    );

    // ④ 真失败：错误码原样透传，不退化成「成功但没发布」。
    worker.send(
        &CommandFrame::new(
            PLUGIN_ID,
            "opc_publish_blog_post",
            serde_json::json!({ "id": "post-9" }),
            5_000,
        )
        .to_frame_request()
        .expect("命令帧应能编成请求")
        .with_id("req-fault"),
    );
    let call = worker.recv_request();
    worker.send_response(
        &FrameResponse::error("TEST_SEAM_WRITE_FAILED", "宿主写库失败")
            .with_id(call.request_id.clone().expect("出站请求必须带 request_id")),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Error, "接缝真失败必须让命令失败");
    assert_eq!(response.code.as_deref(), Some("TEST_SEAM_WRITE_FAILED"));
    assert_eq!(response.request_id.as_deref(), Some("req-fault"));

    // ⑤ 缺 `id`：命令层就拒，**不发接缝帧**（下一条从 stdout 读到的必须是本命令的错误回执）。
    worker.send(
        &CommandFrame::new(PLUGIN_ID, "opc_publish_blog_post", serde_json::json!({}), 5_000)
            .to_frame_request()
            .expect("命令帧应能编成请求")
            .with_id("req-no-id"),
    );
    let response = worker.recv_response();
    assert_eq!(
        response.code.as_deref(),
        Some(error_codes::PLUGIN_COMMAND_INVALID_ARGS),
        "缺 id 属命令层参数错误，不该换来一次注定失败的帧：{response:?}"
    );
    assert_eq!(response.request_id.as_deref(), Some("req-no-id"));

    worker.send(&FrameRequest::new(ops::SHUTDOWN, serde_json::json!({})));
    assert_eq!(worker.recv_response().kind, ResponseKind::Success);
    worker.close_stdin();
    assert!(worker.child.wait().expect("等待 worker 退出").success());
}

/// `opc_delete_blog_post` 的帧级实证（#180-①：删除写 op）。
///
/// 锁五件只有真管道能钉住的跨层口径：
/// 1. 出站帧的 seam / op 逐字 = `opc.store` / `delete_blog_post`（写错会落
///    `SEAM_CALL_UNSUPPORTED`，而前端只看得见「删除失败」）；
/// 2. 出站 `args` **只有 `id` 一个键**、值与命令入参逐字相同（不套 `request`、不带其它字段）；
/// 3. **三态全部走 payload**：`"removed"` / `"not_found"` / `"protected"` 都是
///    `ResponseKind::Success`，值原样成为命令出参（不包装、不改写）——「本来没有」与
///    「不许删」都是业务读数，借错误码表达就等于让前端把三件事读成一件；
/// 4. 宿主侧真失败 ⇒ 原样透传错误码；
/// 5. 缺 `id` ⇒ 命令层就拒，**不发接缝帧**。
#[test]
fn command_frame_deletes_a_blog_post_via_host_seam() {
    let mut worker = Worker::spawn();

    // ① + ② + ③（`removed`）。
    worker.send(
        &CommandFrame::new(
            PLUGIN_ID,
            "opc_delete_blog_post",
            serde_json::json!({ "id": "post-7" }),
            5_000,
        )
        .to_frame_request()
        .expect("命令帧应能编成请求")
        .with_id("req-delete"),
    );
    let call = worker.recv_request();
    assert_eq!(call.op, ops::CALL_SEAM, "写面同样经 call_seam 出站");
    assert_eq!(call.args["seam"], "opc.store");
    assert_eq!(call.args["op"], "delete_blog_post", "op 名必须逐字");
    let args = call.args["args"].as_object().expect("出站 args 应是对象");
    assert_eq!(
        args.keys().cloned().collect::<Vec<String>>(),
        vec!["id".to_string()],
        "删除入参只有 id 一个键：{args:?}"
    );
    assert_eq!(args["id"], "post-7", "id 必须逐字透传，插件侧不做任何加工");
    worker.send_response(
        &FrameResponse::success(serde_json::json!("removed"))
            .with_id(call.request_id.clone().expect("出站请求必须带 request_id")),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Success, "命中应回成功：{response:?}");
    assert_eq!(response.request_id.as_deref(), Some("req-delete"));
    let reply = response.value.expect("命令回执必须带 value");
    assert_eq!(reply["plugin_id"], PLUGIN_ID);
    assert_eq!(reply["value"], serde_json::json!("removed"), "三态串逐字透出，不建包装对象");

    // ③ 另外两态：仍然 Success，值逐字 —— **不许**借错误码。
    for (id, want) in [("gone", "not_found"), ("keep-me", "protected")] {
        worker.send(
            &CommandFrame::new(
                PLUGIN_ID,
                "opc_delete_blog_post",
                serde_json::json!({ "id": id }),
                5_000,
            )
            .to_frame_request()
            .expect("命令帧应能编成请求")
            .with_id(format!("req-{want}")),
        );
        let call = worker.recv_request();
        assert_eq!(call.args["op"], "delete_blog_post");
        assert_eq!(call.args["args"]["id"], id, "id 必须逐字");
        worker.send_response(
            &FrameResponse::success(serde_json::json!(want))
                .with_id(call.request_id.clone().expect("出站请求必须带 request_id")),
        );
        let response = worker.recv_response();
        assert_eq!(
            response.kind,
            ResponseKind::Success,
            "`{want}` 是正常结果，落成错误就等于让前端把三件事读成一件：{response:?}"
        );
        assert_eq!(response.code, None, "成功帧不该带错误码：{response:?}");
        assert_eq!(response.value.expect("命令回执必须带 value")["value"], serde_json::json!(want));
    }

    // ④ 真失败：错误码原样透传。
    worker.send(
        &CommandFrame::new(
            PLUGIN_ID,
            "opc_delete_blog_post",
            serde_json::json!({ "id": "post-9" }),
            5_000,
        )
        .to_frame_request()
        .expect("命令帧应能编成请求")
        .with_id("req-delete-fault"),
    );
    let call = worker.recv_request();
    worker.send_response(
        &FrameResponse::error("TEST_SEAM_DELETE_FAILED", "宿主删库失败")
            .with_id(call.request_id.clone().expect("出站请求必须带 request_id")),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Error, "接缝真失败必须让命令失败");
    assert_eq!(response.code.as_deref(), Some("TEST_SEAM_DELETE_FAILED"));
    assert_eq!(response.request_id.as_deref(), Some("req-delete-fault"));

    // ⑤ 缺 `id`：命令层就拒，**不发接缝帧**（下一条从 stdout 读到的必须是错误回执）。
    worker.send(
        &CommandFrame::new(PLUGIN_ID, "opc_delete_blog_post", serde_json::json!({}), 5_000)
            .to_frame_request()
            .expect("命令帧应能编成请求")
            .with_id("req-delete-no-id"),
    );
    let response = worker.recv_response();
    assert_eq!(
        response.code.as_deref(),
        Some(error_codes::PLUGIN_COMMAND_INVALID_ARGS),
        "缺 id 属命令层参数错误，不该换来一次注定失败的帧：{response:?}"
    );
    assert_eq!(response.request_id.as_deref(), Some("req-delete-no-id"));

    worker.send(&FrameRequest::new(ops::SHUTDOWN, serde_json::json!({})));
    assert_eq!(worker.recv_response().kind, ResponseKind::Success);
    worker.close_stdin();
    assert!(worker.child.wait().expect("等待 worker 退出").success());
}

/// `opc_create_blog_post` 的帧级实证（#180-②：创建写 op，首个带**正文**的入参）。
///
/// 锁五件只有真管道能钉住的跨层口径：
/// 1. 出站帧的 seam / op 逐字 = `opc.store` / `create_blog_post`；
/// 2. 命令入参是**平铺**的五个字段，出站 `args` 是**嵌一层**的 `{ "request": {五个字段} }`
///    —— 多余键不转发，五个字段逐字（含正文，字节闸在宿主侧）；
/// 3. 出参 = 接缝的列裁剪视图**逐字**（10 个 camelCase 键、无 `content`），
///    且带 `id`（调用方后续发布 / 删除的句柄）；
/// 4. 形状不符（缺字段 / tags 非字符串数组）⇒ 命令层拒，**不发帧**；
/// 5. 宿主侧真失败（含正文超字节预算 ⇒ `SEAM_CALL_INVALID_ARGS`）⇒ 原样透传错误码。
#[test]
fn command_frame_creates_a_blog_post_via_host_seam() {
    let mut worker = Worker::spawn();

    // ① + ② + ③
    worker.send(
        &CommandFrame::new(
            PLUGIN_ID,
            "opc_create_blog_post",
            serde_json::json!({
                "title": "第一篇",
                "slug": "first-post",
                "excerpt": "摘要",
                "content": "正文若干字",
                "tags": ["opc", "blog"],
                // 多余键：形状闸只转发那五个（接缝 args 的形状是契约）。
                "extra": true,
            }),
            5_000,
        )
        .to_frame_request()
        .expect("命令帧应能编成请求")
        .with_id("req-create"),
    );
    let call = worker.recv_request();
    assert_eq!(call.op, ops::CALL_SEAM, "写面同样经 call_seam 出站");
    assert_eq!(call.args["seam"], "opc.store");
    assert_eq!(call.args["op"], "create_blog_post", "op 名必须逐字");
    let args = call.args["args"].as_object().expect("出站 args 应是对象");
    assert_eq!(
        args.keys().cloned().collect::<Vec<String>>(),
        vec!["request".to_string()],
        "DTO 入参必须嵌一层 `request`（先例 knowledge.vector）：{args:?}"
    );
    let request = args["request"].as_object().expect("request 应是对象");
    let mut request_keys: Vec<String> = request.keys().cloned().collect();
    request_keys.sort();
    assert_eq!(
        request_keys,
        ["content", "excerpt", "slug", "tags", "title"],
        "只转发 DTO 的五个字段（多余键不得过线）：{request:?}"
    );
    assert_eq!(request["title"], "第一篇");
    assert_eq!(request["slug"], "first-post");
    assert_eq!(request["excerpt"], "摘要");
    assert_eq!(request["content"], "正文若干字", "正文必须逐字过线（字节闸在宿主）");
    assert_eq!(request["tags"], serde_json::json!(["opc", "blog"]));

    // 宿主回的视图原样成为命令出参（10 个 camelCase 键，无正文）。
    let view = seam_row("post-new", 0);
    worker.send_response(
        &FrameResponse::success(view.clone())
            .with_id(call.request_id.clone().expect("出站请求必须带 request_id")),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Success, "创建应回成功：{response:?}");
    assert_eq!(response.request_id.as_deref(), Some("req-create"));
    let reply = response.value.expect("命令回执必须带 value");
    assert_eq!(reply["plugin_id"], PLUGIN_ID);
    assert_eq!(reply["value"], view, "出参必须逐字透传接缝视图，不建包装对象");
    let mut keys: Vec<String> =
        reply["value"].as_object().expect("视图应是对象").keys().cloned().collect();
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
            "viewCount",
        ]
    );
    assert!(reply["value"].get("content").is_none(), "正文不过线");
    assert_eq!(reply["value"]["id"], "post-new", "创建必须回后端给的 id");

    // ④ 形状不符：不发帧，直接落命令层参数错误。
    for bad in [
        serde_json::json!({ "title": "t", "slug": "s", "excerpt": "e", "content": "c" }),
        serde_json::json!({ "title": "t", "slug": "s", "excerpt": "e", "content": "c",
                            "tags": "opc" }),
    ] {
        worker.send(
            &CommandFrame::new(PLUGIN_ID, "opc_create_blog_post", bad, 5_000)
                .to_frame_request()
                .expect("命令帧应能编成请求"),
        );
        let response = worker.recv_response();
        assert_eq!(
            response.code.as_deref(),
            Some(error_codes::PLUGIN_COMMAND_INVALID_ARGS),
            "形状不符必须在命令层拒（不发帧）：{response:?}"
        );
    }

    // ⑤ 宿主的 Validation（正文超预算）⇒ `SEAM_CALL_INVALID_ARGS` 原样透传。
    worker.send(
        &CommandFrame::new(
            PLUGIN_ID,
            "opc_create_blog_post",
            serde_json::json!({ "title": "t", "slug": "s", "excerpt": "e", "content": "c",
                                "tags": [] }),
            5_000,
        )
        .to_frame_request()
        .expect("命令帧应能编成请求")
        .with_id("req-create-over-budget"),
    );
    let call = worker.recv_request();
    worker.send_response(
        &FrameResponse::error(error_codes::SEAM_CALL_INVALID_ARGS, "content 越出契约上限")
            .with_id(call.request_id.clone().expect("出站请求必须带 request_id")),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Error);
    assert_eq!(response.code.as_deref(), Some(error_codes::SEAM_CALL_INVALID_ARGS));
    assert_eq!(response.request_id.as_deref(), Some("req-create-over-budget"));

    worker.send(&FrameRequest::new(ops::SHUTDOWN, serde_json::json!({})));
    assert_eq!(worker.recv_response().kind, ResponseKind::Success);
    worker.close_stdin();
    assert!(worker.child.wait().expect("等待 worker 退出").success());
}

/// #36 批 2：三条 id / 枚举形态写命令的帧级实证。
///
/// 锁五件只有真管道能钉住的跨层口径：
/// 1. 出站 seam / op 逐字 = `opc.store` / 三条 op 名（写错落 `SEAM_CALL_UNSUPPORTED`，
///    而前端只看得见「操作失败」）；
/// 2. 会计域 `opc_transition_invoice` 已迁 `plugin-opc-accounting`，此处不再持有 owner；
/// 3. `opc_update_content_asset` 出站是 `{"id", "request"}` **两层**（DTO 必须嵌层），
///    线格式用 camelCase `contentType`（禁区 13），缺键不补 `null`（`None` = 该列不动）；
/// 4. 出参三档全部走 payload：`{"transitioned": {…}}` / `{"updated": {…}}` / `"removed"` /
///    `"not_found"` 都是 `ResponseKind::Success`，值原样成为命令出参；
/// 5. 缺 `id` / 缺 `status` / 字段类型不符 ⇒ 命令层就拒，**不发接缝帧**。
#[test]
fn command_frame_batch2_writes_go_through_host_seam() {
    let mut worker = Worker::spawn();

    // ① 内容资产更新：`{"id", "request"}` 嵌层 + camelCase 线格式。
    worker.send(
        &CommandFrame::new(
            PLUGIN_ID,
            "opc_update_content_asset",
            serde_json::json!({
                "id": "asset-1",
                "title": "新标题",
                "contentType": "video",
                "body": "正文",
                "tags": ["a", "b"],
                "status": "draft",
            }),
            5_000,
        )
        .to_frame_request()
        .expect("命令帧应能编成请求")
        .with_id("req-update"),
    );
    let call = worker.recv_request();
    assert_eq!(call.args["seam"], "opc.store");
    assert_eq!(call.args["op"], "update_content_asset", "op 名必须逐字");
    let args = call.args["args"].as_object().expect("出站 args 应是对象");
    let mut keys: Vec<String> = args.keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        ["id".to_string(), "request".to_string()],
        "更新入参是 DTO，必须嵌 request 一层：{args:?}"
    );
    assert_eq!(args["id"], "asset-1");
    let request = args["request"].as_object().expect("request 应是对象");
    let mut req_keys: Vec<String> = request.keys().cloned().collect();
    req_keys.sort();
    assert_eq!(
        req_keys,
        ["body", "contentType", "status", "tags", "title"].map(str::to_string).to_vec(),
        "五个可选字段原样转发，线格式是 camelCase（禁区 13）：{request:?}"
    );
    assert_eq!(request["contentType"], "video", "snake_case 不得出现在线格式里");
    assert_eq!(request["tags"], serde_json::json!(["a", "b"]), "tags 逐字转发");
    let asset = serde_json::json!({
        "id": "asset-1",
        "title": "新标题",
        "contentType": "video",
        "status": "draft",
        "tags": ["a", "b"],
        "createdAt": 1_700_000_000_i64,
    });
    worker.send_response(
        &FrameResponse::success(serde_json::json!({ "updated": asset }))
            .with_id(call.request_id.clone().expect("出站请求必须带 request_id")),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Success, "更新应回成功：{response:?}");
    let reply = response.value.expect("命令回执必须带 value");
    assert_eq!(reply["plugin_id"], PLUGIN_ID);
    assert_eq!(
        reply["value"],
        serde_json::json!({ "updated": asset }),
        "带数据的出参是外部标记枚举，逐字透传、不拆包"
    );

    // ② 只给一个字段：缺键不补 `null`（`None` 的语义是「该列不动」）。
    worker.send(
        &CommandFrame::new(
            PLUGIN_ID,
            "opc_update_content_asset",
            serde_json::json!({ "id": "asset-2", "title": "仅标题" }),
            5_000,
        )
        .to_frame_request()
        .expect("命令帧应能编成请求"),
    );
    let call = worker.recv_request();
    let request = call.args["args"]["request"].as_object().expect("request 应是对象");
    assert_eq!(
        request.keys().cloned().collect::<Vec<String>>(),
        vec!["title".to_string()],
        "缺键不补 null，只转发真正给出的列：{request:?}"
    );
    worker.send_response(
        &FrameResponse::success(serde_json::json!("not_found"))
            .with_id(call.request_id.clone().expect("出站请求必须带 request_id")),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Success, "缺行仍是成功 + payload：{response:?}");
    assert_eq!(response.code, None);
    assert_eq!(response.value.expect("命令回执必须带 value")["value"], "not_found");

    // ③ 内容资产删除：两态逐字透传。
    for (id, want, tag) in
        [("asset-3", "removed", "req-asset-remove"), ("gone", "not_found", "req-asset-absent")]
    {
        worker.send(
            &CommandFrame::new(
                PLUGIN_ID,
                "opc_delete_content_asset",
                serde_json::json!({ "id": id }),
                5_000,
            )
            .to_frame_request()
            .expect("命令帧应能编成请求")
            .with_id(tag),
        );
        let call = worker.recv_request();
        assert_eq!(call.args["seam"], "opc.store");
        assert_eq!(call.args["op"], "delete_content_asset", "op 名必须逐字");
        let args = call.args["args"].as_object().expect("出站 args 应是对象");
        assert_eq!(
            args.keys().cloned().collect::<Vec<String>>(),
            vec!["id".to_string()],
            "删除入参只有 id 一个键：{args:?}"
        );
        assert_eq!(args["id"], id);
        worker.send_response(
            &FrameResponse::success(serde_json::json!(want))
                .with_id(call.request_id.clone().expect("出站请求必须带 request_id")),
        );
        let response = worker.recv_response();
        assert_eq!(response.kind, ResponseKind::Success, "两态都走 payload：{response:?}");
        assert_eq!(response.code, None);
        assert_eq!(
            response.value.expect("命令回执必须带 value")["value"],
            serde_json::json!(want),
            "两态串逐字透出，不建包装对象"
        );
    }

    // ④ 发布计划取消（硬删）：两态逐字透传。
    for (id, want) in [("sched-1", "removed"), ("gone", "not_found")] {
        worker.send(
            &CommandFrame::new(
                PLUGIN_ID,
                "opc_cancel_publish_schedule",
                serde_json::json!({ "id": id }),
                5_000,
            )
            .to_frame_request()
            .expect("命令帧应能编成请求"),
        );
        let call = worker.recv_request();
        assert_eq!(call.args["seam"], "opc.store");
        assert_eq!(call.args["op"], "cancel_publish_schedule", "op 名必须逐字");
        let args = call.args["args"].as_object().expect("出站 args 应是对象");
        assert_eq!(
            args.keys().cloned().collect::<Vec<String>>(),
            vec!["id".to_string()],
            "取消入参只有 id 一个键：{args:?}"
        );
        assert_eq!(args["id"], id);
        worker.send_response(
            &FrameResponse::success(serde_json::json!(want))
                .with_id(call.request_id.clone().expect("出站请求必须带 request_id")),
        );
        let response = worker.recv_response();
        assert_eq!(response.kind, ResponseKind::Success, "两态都走 payload：{response:?}");
        assert_eq!(response.code, None);
        assert_eq!(response.value.expect("命令回执必须带 value")["value"], serde_json::json!(want));
    }

    // ④ 形状闸：缺 id / 缺 status / 字段类型不符 ⇒ 命令层直接拒，**不发接缝帧**
    //    （下一条从 stdout 读到的必须是本命令的错误回执）。
    for (command, input) in [
        ("opc_update_content_asset", serde_json::json!({ "title": "t" })),
        ("opc_update_content_asset", serde_json::json!({ "id": "a", "title": 1 })),
        ("opc_update_content_asset", serde_json::json!({ "id": "a", "tags": "opc" })),
        ("opc_delete_content_asset", serde_json::json!({})),
        ("opc_cancel_publish_schedule", serde_json::json!({})),
    ] {
        worker.send(
            &CommandFrame::new(PLUGIN_ID, command, input.clone(), 5_000)
                .to_frame_request()
                .expect("命令帧应能编成请求"),
        );
        let response = worker.recv_response();
        assert_eq!(
            response.code.as_deref(),
            Some(error_codes::PLUGIN_COMMAND_INVALID_ARGS),
            "{command} 的形状不符必须在命令层拒（{input}）：{response:?}"
        );
    }

    worker.send(&FrameRequest::new(ops::SHUTDOWN, serde_json::json!({})));
    assert_eq!(worker.recv_response().kind, ResponseKind::Success);
    worker.close_stdin();
    assert!(worker.child.wait().expect("等待 worker 退出").success());
}

/// #36 批 3 的单次往返：发命令 → 取出站帧 → 校验 seam / op → 回固定 payload → 校验逐字透传。
///
/// 返回出站 `args`（已断言是对象），供各处再钉键集合 —— 出站形状是这一批的主要判据
/// （嵌层 / camelCase / 缺键不补 null），入参键名写错不会在插件里报错，只会让宿主 serde
/// 落 `SEAM_CALL_INVALID_ARGS`，只有帧级断言能在本地暴露。
fn batch3_roundtrip(
    worker: &mut Worker,
    command: &str,
    input: serde_json::Value,
    expected_op: &str,
    reply: serde_json::Value,
) -> serde_json::Map<String, serde_json::Value> {
    worker.send(
        &CommandFrame::new(PLUGIN_ID, command, input, 5_000)
            .to_frame_request()
            .expect("命令帧应能编成请求"),
    );
    let call = worker.recv_request();
    assert_eq!(call.op, ops::CALL_SEAM, "{command} 必须经 call_seam 出站");
    assert_eq!(call.args["seam"], "opc.store");
    assert_eq!(call.args["op"], expected_op, "{command} 的 op 名必须逐字");
    let args = call.args["args"].as_object().cloned().expect("出站 args 应是对象");
    worker.send_response(
        &FrameResponse::success(reply.clone())
            .with_id(call.request_id.clone().expect("出站请求必须带 request_id")),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Success, "{command} 应回成功：{response:?}");
    assert_eq!(response.code, None, "成功帧不该带错误码：{response:?}");
    let value = response.value.expect("命令回执必须带 value");
    assert_eq!(value["plugin_id"], PLUGIN_ID);
    assert_eq!(value["value"], reply, "{command} 的出参必须逐字透传");
    args
}

/// #36 批 3：七条写命令 + 一条只读报表的帧级实证。
///
/// 锁四件只有真管道能钉住的跨层口径：
/// 1. 出站 seam / op 逐字 = `opc.store` / 八条 op 名（写错时宿主回 `SEAM_CALL_UNSUPPORTED`，
///    而调用方只看得见「操作失败」）；
/// 2. 七条写命令的出站是 `{"request": {…}}` **两层**、camelCase 线格式（禁区 13）；
///    `get_financial_report` 是**散标量** `{"period": …}`，不套壳（套错会被
///    `deserialize_arg` 判 `SEAM_CALL_INVALID_ARGS`）；
/// 3. 出参是契约的列裁剪视图 / 视图数组，逐字透传、不拆包不改名；
/// 4. 形状闸：必需键缺失 / 类型不符 ⇒ 命令层直接拒，**不发接缝帧**。
#[test]
fn command_frame_batch3_creates_go_through_host_seam() {
    let mut worker = Worker::spawn();

    // ① 创建客户：`customer_type` → `customerType`；未给出的键不补 null。
    let customer = serde_json::json!({
        "id": "cust-1",
        "name": "张三",
        "email": "z@example.com",
        "phone": null,
        "company": null,
        "status": "lead",
        "totalRevenue": 0.0,
        "createdAt": 1_700_000_000_i64,
    });
    let args = batch3_roundtrip(
        &mut worker,
        "opc_create_customer",
        serde_json::json!({ "name": "张三", "email": "z@example.com", "customerType": "Business" }),
        "create_customer",
        customer,
    );
    let request = args["request"].as_object().expect("request 应是对象");
    let mut keys = request.keys().cloned().collect::<Vec<String>>();
    keys.sort();
    assert_eq!(
        keys,
        ["customerType", "email", "name"].map(str::to_string).to_vec(),
        "只转发真正给出的键，缺键不补 null：{request:?}"
    );
    assert_eq!(request["customerType"], "Business");

    // ③ 项目与里程碑：`project_id` → `projectId`；里程碑 id / 状态由宿主设。
    let project = serde_json::json!({
        "id": "proj-1",
        "title": "官网改版",
        "status": "planning",
        "budget": 50000.0,
        "currency": "CNY",
        "milestoneTotal": 1,
        "milestoneCompleted": 0,
        "createdAt": 1_700_000_000_i64,
    });
    let args = batch3_roundtrip(
        &mut worker,
        "opc_create_project",
        serde_json::json!({ "title": "官网改版", "customerId": "cust-1", "budget": 50000.0 }),
        "create_project",
        project.clone(),
    );
    let request = args["request"].as_object().expect("request 应是对象");
    assert_eq!(request["customerId"], "cust-1");
    assert_eq!(request["budget"], 50000.0);

    let args = batch3_roundtrip(
        &mut worker,
        "opc_add_milestone",
        serde_json::json!({ "projectId": "proj-1", "title": "上线", "dueAt": 1_700_400_000_i64 }),
        "add_milestone",
        project,
    );
    let request = args["request"].as_object().expect("request 应是对象");
    let mut keys = request.keys().cloned().collect::<Vec<String>>();
    keys.sort();
    assert_eq!(
        keys,
        ["dueAt", "projectId", "title"].map(str::to_string).to_vec(),
        "里程碑入参只有这三个键（id / status 由宿主生成）：{request:?}"
    );

    // ④ 落地页：缺省 description 不补键。
    let page = serde_json::json!({
        "id": "page-1",
        "title": "落地页",
        "slug": "lp",
        "published": false,
        "createdAt": 1_700_000_000_i64,
    });
    let args = batch3_roundtrip(
        &mut worker,
        "opc_create_landing_page",
        serde_json::json!({ "title": "落地页", "slug": "lp", "content": "# 正文" }),
        "create_landing_page",
        page,
    );
    let request = args["request"].as_object().expect("request 应是对象");
    let mut keys = request.keys().cloned().collect::<Vec<String>>();
    keys.sort();
    assert_eq!(
        keys,
        ["content", "slug", "title"].map(str::to_string).to_vec(),
        "缺省 description 不补键：{request:?}"
    );

    // ⑤ 财务报表：**散标量**，不套 request 壳。
    let report = serde_json::json!({
        "period": "2026-07",
        "totalRevenue": 1000.0,
        "pendingRevenue": 0.0,
        "refundedAmount": 0.0,
        "activeProjects": 2,
        "newCustomers": 3,
        "netProfit": 900.0,
        "investableAmount": 600.0,
        "churnedCustomers": 0,
        "pretaxRevenue": 950.0,
        "riskLevel": "低",
        "recommendedAmount": 300.0,
        "advice": "可追加投资",
    });
    let args = batch3_roundtrip(
        &mut worker,
        "opc_get_financial_report",
        serde_json::json!({ "period": "2026-07" }),
        "get_financial_report",
        report,
    );
    assert_eq!(
        args.keys().cloned().collect::<Vec<String>>(),
        vec!["period".to_string()],
        "报表入参是散标量，不套 request：{args:?}"
    );
    assert_eq!(args["period"], "2026-07");

    // ⑥ 内容资产：`content_type` → `contentType` 线格式；tags 逐字转发。
    let asset = serde_json::json!({
        "id": "asset-1",
        "title": "文章",
        "contentType": "article",
        "status": "draft",
        "tags": ["opc"],
        "createdAt": 1_700_000_000_i64,
    });
    let args = batch3_roundtrip(
        &mut worker,
        "opc_create_content_asset",
        serde_json::json!({
            "title": "文章",
            "contentType": "article",
            "body": "正文",
            "tags": ["opc"],
        }),
        "create_content_asset",
        asset,
    );
    assert_eq!(args["request"]["contentType"], "article", "snake_case 不得出现在线格式里");
    assert_eq!(args["request"]["tags"], serde_json::json!(["opc"]));

    // ⑦ 发布计划：`contentRefId` 是**被发布内容**的主键（不是本行 id）。
    let schedule = serde_json::json!({
        "id": "sched-1",
        "contentRefType": "blog_post",
        "contentRefId": "post-1",
        "status": "pending",
        "scheduledAt": 1_700_400_000_i64,
        "publishedAt": null,
    });
    let args = batch3_roundtrip(
        &mut worker,
        "opc_create_publish_schedule",
        serde_json::json!({
            "contentRefType": "blog_post",
            "contentRefId": "post-1",
            "scheduledAt": 1_700_400_000_i64,
        }),
        "create_publish_schedule",
        schedule,
    );
    let request = args["request"].as_object().expect("request 应是对象");
    let mut keys = request.keys().cloned().collect::<Vec<String>>();
    keys.sort();
    assert_eq!(
        keys,
        ["contentRefId", "contentRefType", "scheduledAt"].map(str::to_string).to_vec(),
        "{request:?}"
    );

    // ⑧ 处理到期计划：无入参 ⇒ 出站 args 是空对象；空数组出参仍是成功（不是错误）。
    let args = batch3_roundtrip(
        &mut worker,
        "opc_process_due_schedules",
        serde_json::json!({}),
        "process_due_schedules",
        serde_json::json!([]),
    );
    assert!(args.is_empty(), "无入参 op 的出站 args 必须是空对象：{args:?}");

    // ⑨ 形状闸：必需键缺失 / 类型不符 ⇒ 命令层直接拒，**不发接缝帧**。
    for (command, input) in [
        ("opc_create_customer", serde_json::json!({ "name": "张三" })),
        (
            "opc_create_customer",
            serde_json::json!({ "name": "张三", "email": "z", "latitude": "北纬30" }),
        ),
        ("opc_create_project", serde_json::json!({ "budget": 1 })),
        ("opc_add_milestone", serde_json::json!({ "title": "上线", "projectId": 3 })),
        ("opc_create_landing_page", serde_json::json!({ "title": "t", "slug": "s" })),
        ("opc_get_financial_report", serde_json::json!({})),
        ("opc_create_content_asset", serde_json::json!({ "title": "t", "contentType": "article" })),
        (
            "opc_create_content_asset",
            serde_json::json!({ "title": "t", "contentType": "article", "body": "b", "tags": "opc" }),
        ),
        (
            "opc_create_publish_schedule",
            serde_json::json!({ "contentRefType": "blog_post", "contentRefId": "p" }),
        ),
        (
            "opc_create_publish_schedule",
            serde_json::json!({
                "contentRefType": "blog_post",
                "contentRefId": "p",
                "scheduledAt": 1.5,
            }),
        ),
    ] {
        worker.send(
            &CommandFrame::new(PLUGIN_ID, command, input.clone(), 5_000)
                .to_frame_request()
                .expect("命令帧应能编成请求"),
        );
        let response = worker.recv_response();
        assert_eq!(
            response.code.as_deref(),
            Some(error_codes::PLUGIN_COMMAND_INVALID_ARGS),
            "{command} 的形状不符必须在命令层拒（{input}）：{response:?}"
        );
    }

    worker.send(&FrameRequest::new(ops::SHUTDOWN, serde_json::json!({})));
    assert_eq!(worker.recv_response().kind, ResponseKind::Success);
    worker.close_stdin();
    assert!(worker.child.wait().expect("等待 worker 退出").success());
}
