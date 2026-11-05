// SPDX-License-Identifier: AGPL-3.0-only
//! 真进程 + 真管道的协议测试：宿主将来就是按这个形态 spawn 它的。
//!
//! 为什么不用进程内替代：`stdin`/`stdout` 的缓冲与 flush、以及「等响应期间对端又发来
//! 请求」的重入语义，只有在真管上才暴露。

use std::io::{BufReader, BufWriter};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use axagent_plugin_proto::{
    AXAGENT_PLUGIN_PROTO_VERSION, CommandFrame, FrameRequest, FrameResponse, PluginDeclaration,
    ResponseKind, error_codes, ops, read_inbound, write_json_frame,
};

struct Worker {
    child: Child,
    /// `Option` 只为一件事：`closing_stdin_is_a_clean_exit` 要真的关掉写端。
    stdin: Option<BufWriter<ChildStdin>>,
    stdout: BufReader<ChildStdout>,
}

/// 与宿主 `PluginWorkerConfig::plugin_id` 对齐的身份（命令帧的身份自检拿它比对）。
const PLUGIN_ID: &str = "plugin.stock";

impl Worker {
    fn spawn() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_axagent-plugin-stock"))
            // 宿主 spawn 时会注入自身身份；命令帧的身份自检依赖它（缺省即空串 ⇒ 任何帧都拒）。
            .env("AXAGENT_PLUGIN_ID", PLUGIN_ID)
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

    /// 读一帧**请求**（worker → 宿主）。
    fn recv_request(&mut self) -> FrameRequest {
        match read_inbound(&mut self.stdout).expect("读帧应成功").as_request().cloned() {
            Some(request) => request,
            None => panic!("期望一帧请求（worker → 宿主），实际是响应"),
        }
    }

    /// 读一帧**响应**（宿主 → worker 的请求被回执）。
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

#[test]
fn describe_declares_the_tool_seam_and_its_outbound_call() {
    let mut worker = Worker::spawn();
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

    let caps: Vec<(&str, &str)> =
        declaration.capabilities.iter().map(|c| (c.seam.as_str(), c.op.as_str())).collect();
    assert!(caps.contains(&("tool.set", "exec")), "必须声明 tool.set 的执行面：{caps:?}");
    assert_eq!(
        declaration.calls,
        vec!["market.quote".to_string(), "workflow.run".to_string(), "analysis.record".to_string(),],
        "出站接缝必须逐条声明，否则宿主 handle_call_seam 直接拒调\
         （行情 + 工作流任务源 + 单股分析记录）"
    );

    worker.send(&FrameRequest::new(ops::SHUTDOWN, serde_json::json!({})));
    assert_eq!(worker.recv_response().kind, ResponseKind::Success);
    worker.close_stdin();
    let status = worker.child.wait().expect("等待 worker 退出");
    assert!(status.success(), "收到 shutdown 并关管道后应以 0 退出，实际 {status:?}");
}

#[test]
fn tool_list_exposes_get_stock_quote() {
    let mut worker = Worker::spawn();
    worker.send(&FrameRequest::new("list", serde_json::json!({})));
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Success);

    let tools = response.value.expect("list 必须带 value");
    let tools = tools.as_array().expect("list 应返回数组");
    assert_eq!(tools.len(), 1, "当前只贡献一个工具：{tools:?}");
    assert_eq!(tools[0]["name"], "get_stock_quote");
    // 字段名是宿主 `RemoteToolDescriptor` 的 camelCase 形态；写错宿主会解析失败。
    assert!(tools[0]["inputSchema"]["required"][0] == "stock_code", "缺少 inputSchema.required");
}

#[test]
fn exec_calls_back_into_the_host_market_seam_and_returns_content() {
    let mut worker = Worker::spawn();
    worker.send(
        &FrameRequest::new(
            "exec",
            serde_json::json!({ "name": "get_stock_quote", "args": { "stock_code": "600519" } }),
        )
        .with_id("req-exec"),
    );

    // ① worker 必须先向宿主发 call_seam（不是自己去取数：凭证/provider 都在宿主侧）。
    let call = worker.recv_request();
    assert_eq!(call.op, ops::CALL_SEAM);
    assert_eq!(call.args["seam"], "market.quote");
    assert_eq!(call.args["op"], "get_quote");
    assert_eq!(call.args["args"]["stock_code"], "600519");
    let call_id = call.request_id.clone().expect("出站请求必须带 request_id");

    // ② 宿主侧行情 provider 的回帧（固定值，证明插件把数据原样转成 content）。
    worker.send_response(
        &FrameResponse::success(serde_json::json!({ "stockCode": "600519", "price": 1500.25 }))
            .with_id(call_id),
    );

    // ③ 工具执行结果回到宿主：`content` 是字符串，`request_id` 原样回填。
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Success);
    assert_eq!(response.request_id.as_deref(), Some("req-exec"));
    let value = response.value.expect("exec 成功必须带 value");
    let content = value["content"].as_str().expect("content 必须是字符串");
    let quote: serde_json::Value = serde_json::from_str(content).expect("content 应是 JSON 字符串");
    assert_eq!(quote["price"], 1500.25, "行情数据必须原样透出：{quote:?}");
    assert_eq!(value["metadata"]["stock_code"], "600519");

    worker.send(&FrameRequest::new(ops::SHUTDOWN, serde_json::json!({})));
    assert_eq!(worker.recv_response().kind, ResponseKind::Success);
    worker.close_stdin();
    assert!(worker.child.wait().expect("等待 worker 退出").success());
}

#[test]
fn market_seam_error_is_propagated_as_a_failed_tool_call() {
    let mut worker = Worker::spawn();
    worker.send(&FrameRequest::new(
        "exec",
        serde_json::json!({ "name": "get_stock_quote", "args": { "stock_code": "000001" } }),
    ));
    let call = worker.recv_request();
    worker.send_response(
        &FrameResponse::error("MARKET_PROVIDER_UNAVAILABLE", "行情源不可用")
            .with_id(call.request_id.clone().expect("带 id")),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Error, "提供方报错不得被包装成成功");
    assert_eq!(response.code.as_deref(), Some("MARKET_PROVIDER_UNAVAILABLE"));
}

#[test]
fn unknown_op_and_unknown_tool_are_explicit_errors() {
    let mut worker = Worker::spawn();
    worker.send(&FrameRequest::new("stock.not_implemented_yet", serde_json::json!({})));
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Error);
    assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_UNKNOWN_OP));

    worker.send(&FrameRequest::new("exec", serde_json::json!({ "name": "nope", "args": {} })));
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Error);
    assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_CALL_FAILED));

    // 缺参数同样显式失败，不按默认值继续。
    worker.send(&FrameRequest::new(
        "exec",
        serde_json::json!({ "name": "get_stock_quote", "args": { "stock_code": "   " } }),
    ));
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Error);
    assert_eq!(response.code.as_deref(), Some(error_codes::SEAM_CALL_INVALID_ARGS));
}

#[test]
fn parse_analysis_intent_runs_the_real_implementation_without_any_seam_call() {
    let mut worker = Worker::spawn();
    worker.send(
        &CommandFrame::new(
            PLUGIN_ID,
            "parse_analysis_intent",
            serde_json::json!({ "input": "分析宁德时代中线" }),
            5_000,
        )
        .to_frame_request()
        .expect("命令帧应能编成请求"),
    );
    // 纯函数命令：不得向宿主发任何帧（若发了 call_seam，这里会读到请求而不是回执）。
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Success);
    let reply = response.value.expect("命令回执必须带 value");
    assert_eq!(reply["value"]["stockCode"], "300750");
    assert_eq!(reply["value"]["timeHorizon"], "mid");
    assert_eq!(reply["value"]["actionType"], "research");
    assert_eq!(reply["value"]["success"], true);

    worker.send(&FrameRequest::new(ops::SHUTDOWN, serde_json::json!({})));
    assert_eq!(worker.recv_response().kind, ResponseKind::Success);
}

/// B9-1：证据质量权重是**纯函数命令** —— 与 `parse_analysis_intent` 同档，
/// 不得向宿主发任何接缝帧（权重表是已校准的本地数值权威，不依赖行情/DB）。
#[test]
fn compute_evidence_weights_runs_without_any_seam_call() {
    let mut worker = Worker::spawn();
    worker.send(
        &CommandFrame::new(
            PLUGIN_ID,
            "compute_evidence_weights",
            serde_json::json!({
                "marketRegime": {
                    "regime": "bull",
                    "confidence": 0.8,
                    "volatility": "normal",
                    "description": "单测固定环境"
                },
                "timeHorizon": "mid",
                "analysts": [{
                    "analystId": "a-market-analyst",
                    "stance": "看多",
                    "bullScore": 70.0,
                    "bearScore": 30.0
                }]
            }),
            5_000,
        )
        .to_frame_request()
        .expect("命令帧应能编成请求"),
    );
    // 纯函数：若向宿主发了 call_seam，这里读到的会是请求而不是回执。
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Success);
    let reply = response.value.expect("命令回执必须带 value");
    let report = &reply["value"];
    assert_eq!(report["timeHorizon"], "mid");
    let weights = report["analystWeights"].as_array().expect("分析师权重必须是数组");
    assert_eq!(weights.len(), 1, "输入一个分析师 ⇒ 输出一个权重：{report:?}");
    assert_eq!(weights[0]["analystId"], "a-market-analyst");
    assert!(
        weights[0]["finalWeight"].as_f64().is_some_and(|w| w > 0.0),
        "最终权重必须是正数：{report:?}"
    );
    assert!(
        report["recommendedAction"].as_str().is_some_and(|a| !a.is_empty()),
        "建议动作不得为空：{report:?}"
    );

    // 缺字段必须显式报参数错误，不按默认值继续算。
    worker.send(
        &CommandFrame::new(PLUGIN_ID, "compute_evidence_weights", serde_json::json!({}), 5_000)
            .to_frame_request()
            .expect("命令帧应能编成请求"),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Error);
    assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_COMMAND_INVALID_ARGS));

    worker.send(&FrameRequest::new(ops::SHUTDOWN, serde_json::json!({})));
    assert_eq!(worker.recv_response().kind, ResponseKind::Success);
}

#[test]
fn closing_stdin_is_a_clean_exit() {
    let mut worker = Worker::spawn();
    // 不发 shutdown，直接关 stdin：宿主停机时靠关管道收尾，worker 不得把它当作错误。
    worker.close_stdin();
    let status = worker.child.wait().expect("等待 worker 退出");
    assert!(status.success(), "EOF 应是干净退出，实际 {status:?}");
}

#[test]
fn command_frame_runs_stock_get_quote_through_the_host_seam() {
    let mut worker = Worker::spawn();
    worker.send(
        &CommandFrame::new(
            PLUGIN_ID,
            "stock_get_quote",
            serde_json::json!({ "stock_code": "600519" }),
            5_000,
        )
        .to_frame_request()
        .expect("命令帧应能编成请求")
        .with_id("req-command"),
    );

    // 命令面与工具面共用同一条出站路径：worker 仍要回调宿主 market.quote 取数。
    let call = worker.recv_request();
    assert_eq!(call.op, ops::CALL_SEAM);
    assert_eq!(call.args["seam"], "market.quote");
    assert_eq!(call.args["op"], "get_quote");
    worker.send_response(
        &FrameResponse::success(serde_json::json!({ "code": "600519", "price": 1500.25 }))
            .with_id(call.request_id.clone().expect("出站请求必须带 request_id")),
    );

    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Success);
    assert_eq!(response.request_id.as_deref(), Some("req-command"));
    let reply = response.value.expect("命令回执必须带 value");
    // `CommandReply` 没有 `rename_all` ⇒ 线格式是 snake_case，与 `CommandFrame` 同形。
    assert_eq!(reply["plugin_id"], PLUGIN_ID, "回执必须回填宿主绑定的身份");
    assert_eq!(reply["command"], "stock_get_quote");
    assert_eq!(reply["value"]["price"], 1500.25, "行情必须原样透出：{reply:?}");

    worker.send(&FrameRequest::new(ops::SHUTDOWN, serde_json::json!({})));
    assert_eq!(worker.recv_response().kind, ResponseKind::Success);
    worker.close_stdin();
    assert!(worker.child.wait().expect("等待 worker 退出").success());
}

#[test]
fn command_frame_identity_mismatch_and_unknown_command_are_explicit_errors() {
    let mut worker = Worker::spawn();

    // 身份不符：帧绑定别的插件 ⇒ 拒执行，且**不得**发起行情调用。
    worker.send(
        &CommandFrame::new("plugin.other", "stock_get_quote", serde_json::json!({}), 5_000)
            .to_frame_request()
            .expect("命令帧应能编成请求"),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Error);
    assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_COMMAND_ID_MISMATCH));

    // 未声明的命令名：显式报未知，不静默成功。
    worker.send(
        &CommandFrame::new(PLUGIN_ID, "stock_not_implemented", serde_json::json!({}), 5_000)
            .to_frame_request()
            .expect("命令帧应能编成请求"),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Error);
    assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_COMMAND_UNKNOWN));

    worker.send(&FrameRequest::new(ops::SHUTDOWN, serde_json::json!({})));
    assert_eq!(worker.recv_response().kind, ResponseKind::Success);
}

/// 任务源取数命令的帧级实证 —— 出站接缝的**参数名口径**与行形状都只有真管道能钉住。
///
/// 与 e2e 安装测试的分工：那边锁「清单 → 投影 → 命令能跑通」，这边锁
/// 「camelCase 入参 → snake_case 接缝参数」的翻译与「引擎状态词 → 中心状态词」的翻译，
/// 以及接缝出错时命令必须**失败**而不是静默回空数组（空数组在任务中心看起来像「今天没活」）。
///
/// B8-1b 建通道，#150 把取数口从 `list_active` 换成 `list_recent`：后者的一页里可以出现
/// 已结束的执行，`finishedAt` 那一条分支从此是实测路径而不再是防御性代码。
#[test]
fn command_frame_lists_recent_workflow_runs_as_task_center_rows() {
    let mut worker = Worker::spawn();
    worker.send(
        &CommandFrame::new(
            PLUGIN_ID,
            "list_recent_workflow_runs",
            serde_json::json!({ "limit": 20, "workflowId": "wf-daily" }),
            5_000,
        )
        .to_frame_request()
        .expect("命令帧应能编成请求")
        .with_id("req-task-rows"),
    );

    let call = worker.recv_request();
    assert_eq!(call.op, ops::CALL_SEAM);
    assert_eq!(call.args["seam"], "workflow.run");
    assert_eq!(call.args["op"], "list_recent");
    // 命令入参是 camelCase（全站约定），接缝入参是 snake_case（宿主 deserialize_optional_arg
    // 按 `workflow_id` 取）⇒ 这层翻译发生在插件侧，写错只会让过滤条件静默失效。
    assert_eq!(call.args["args"]["limit"], 20);
    assert_eq!(
        call.args["args"]["workflow_id"], "wf-daily",
        "接缝参数必须是 workflow_id：{:?}",
        call.args
    );

    worker.send_response(
        &FrameResponse::success(serde_json::json!({
            "executions": [{
                "executionId": "exec-9",
                "workflowId": "wf-daily",
                // 引擎用英式 `cancelled`，中心用美式 `canceled`
                "status": "cancelled",
                "currentNodeId": null,
                "ownerPluginId": PLUGIN_ID,
                "createdAt": 1_700_000_000_000_i64,
                "updatedAt": 1_700_000_009_000_i64,
                // 终态行的结束时刻由宿主登记表盖（不是 `updatedAt`）⇒ 必须原样透出
                "finishedAt": 1_700_000_008_000_i64,
            }],
            "total": 1,
        }))
        .with_id(call.request_id.clone().expect("出站请求必须带 request_id")),
    );

    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Success);
    assert_eq!(response.request_id.as_deref(), Some("req-task-rows"));
    let reply = response.value.expect("命令回执必须带 value");
    assert_eq!(reply["plugin_id"], PLUGIN_ID);
    let rows = reply["value"].as_array().expect("任务源取数必须回数组");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["nativeId"], "exec-9");
    assert_eq!(rows[0]["name"], "wf-daily · exec-9");
    assert_eq!(rows[0]["state"], "canceled", "状态词表翻译必须发生在插件侧：{:?}", rows[0]);
    assert_eq!(rows[0]["trigger"], "manual");
    assert_eq!(rows[0]["startedAt"], 1_700_000_000_000_i64);
    assert_eq!(
        rows[0]["finishedAt"], 1_700_000_008_000_i64,
        "结束时刻必须透传宿主的值：{}",
        rows[0]
    );
    // 缺省项是「不出现」而不是 null：宿主泛型适配器按字段在不在决定耗时列与可否打开。
    // 耗时**不在这里算** —— 两端点相减只发生在宿主 `resolveDurationMs` 一处（不留第二套口径）。
    for absent in ["durationMs", "deepLink"] {
        assert!(rows[0].get(absent).is_none(), "缺省字段 `{absent}` 不得进帧：{}", rows[0]);
    }
    assert_eq!(rows[0]["raw"]["updatedAt"], 1_700_000_009_000_i64, "引擎视图原样留在 raw");

    // 接缝报错 ⇒ 命令失败并**原样透传错误码**，不得退化成空列表。
    worker.send(
        &CommandFrame::new(PLUGIN_ID, "list_recent_workflow_runs", serde_json::json!({}), 5_000)
            .to_frame_request()
            .expect("命令帧应能编成请求")
            .with_id("req-task-rows-2"),
    );
    let second_call = worker.recv_request();
    // 不传 limit ⇒ 插件用与宿主一致的缺省档 50
    assert_eq!(second_call.args["args"]["limit"], 50, "缺省档应与宿主 list_recent 的缺省一致");
    assert!(
        second_call.args["args"].get("workflow_id").is_none()
            || second_call.args["args"]["workflow_id"].is_null(),
        "未指定 workflowId ⇒ 原样不带过滤：{:?}",
        second_call.args
    );
    worker.send_response(
        &FrameResponse::error("TEST_WORKFLOW_RUN_UNAVAILABLE", "宿主引擎没起来")
            .with_id(second_call.request_id.clone().expect("出站请求必须带 request_id")),
    );
    let failed = worker.recv_response();
    assert_eq!(failed.kind, ResponseKind::Error);
    assert_eq!(failed.code.as_deref(), Some("TEST_WORKFLOW_RUN_UNAVAILABLE"));
    assert_eq!(failed.request_id.as_deref(), Some("req-task-rows-2"));

    worker.send(&FrameRequest::new(ops::SHUTDOWN, serde_json::json!({})));
    assert_eq!(worker.recv_response().kind, ResponseKind::Success);
    worker.close_stdin();
    assert!(worker.child.wait().expect("等待 worker 退出").success());
}

/// 单股分析记录取数命令的帧级实证（#149 第 1 步：宿主查询接缝 `analysis.record`）。
///
/// 锁三件事，都是「只有真管道能钉住」的跨层口径：
/// 1. 出站帧的 seam / op 逐字是 `analysis.record` / `list`（接缝名写错 ⇒ 宿主取不到 provider）；
/// 2. 命令入参 `limit` / `offset` **原样透传**（插件不钳位：越界由宿主按契约区间拒收，
///    缺省也交给宿主 ⇒ 插件侧没有第二个 50 / 200）；
/// 3. 行形状：状态词表翻译（`completed` → `succeeded`）、根记录收窄（`parentAnalysisId`
///    非空的行被剔除）、结束时刻只取 `completedAt`（不拿 `updatedAt` 倒推）、
///    `deepLink` / `durationMs` 缺省不出键。
#[test]
fn command_frame_lists_stock_analysis_records_via_host_seam() {
    let mut worker = Worker::spawn();
    worker.send(
        &CommandFrame::new(
            PLUGIN_ID,
            "list_stock_analysis_records",
            serde_json::json!({ "limit": 20, "offset": 40 }),
            5_000,
        )
        .to_frame_request()
        .expect("命令帧应能编成请求")
        .with_id("req-analysis-rows"),
    );

    let call = worker.recv_request();
    assert_eq!(call.op, ops::CALL_SEAM);
    assert_eq!(call.args["seam"], "analysis.record");
    assert_eq!(call.args["op"], "list");
    assert_eq!(call.args["args"]["limit"], 20);
    assert_eq!(call.args["args"]["offset"], 40, "分页偏移必须原样透传：{:?}", call.args);

    worker.send_response(
        &FrameResponse::success(serde_json::json!({
            "records": [
                {
                    "id": "analysis-1",
                    "stockCode": "600519",
                    "stockName": "贵州茅台",
                    "analysisDate": "2026-11-05",
                    "status": "completed",
                    "decisionAction": "BUY",
                    "decisionPositionState": "HOLDING",
                    "decisionPositionPct": 12.5,
                    "analysisKind": "live",
                    "asOfDate": null,
                    "parentAnalysisId": null,
                    "templateId": "stock-analysis",
                    "templateVersion": 143,
                    "decisionTimeHorizon": "short",
                    "decisionHorizonSource": "branch_pick",
                    "createdAt": 1_700_000_000_000_i64,
                    "updatedAt": 1_700_000_900_000_i64,
                    "completedAt": 1_700_000_050_000_i64,
                    "executionQuality": 7,
                },
                {
                    // replay 子行：必须被根记录收窄剔除（宿主适配器同一条判据）。
                    "id": "analysis-2",
                    "stockCode": "600519",
                    "stockName": "贵州茅台",
                    "analysisDate": "2026-10-01",
                    "status": "completed",
                    "decisionAction": "HOLD",
                    "decisionPositionState": null,
                    "decisionPositionPct": null,
                    "analysisKind": "replay",
                    "asOfDate": "2026-10-01",
                    "parentAnalysisId": "analysis-1",
                    "templateId": "stock-analysis",
                    "templateVersion": 143,
                    "decisionTimeHorizon": "short",
                    "decisionHorizonSource": "branch_pick",
                    "createdAt": 1_700_000_000_000_i64,
                    "updatedAt": 1_700_000_050_000_i64,
                    "completedAt": 1_700_000_050_000_i64,
                    "executionQuality": null,
                },
            ],
            "total": 2,
        }))
        .with_id(call.request_id.clone().expect("出站请求必须带 request_id")),
    );

    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Success);
    assert_eq!(response.request_id.as_deref(), Some("req-analysis-rows"));
    let reply = response.value.expect("命令回执必须带 value");
    assert_eq!(reply["plugin_id"], PLUGIN_ID);
    let rows = reply["value"].as_array().expect("任务源取数必须回数组");
    assert_eq!(rows.len(), 1, "replay 子行必须被剔除：{rows:?}");
    assert_eq!(rows[0]["nativeId"], "analysis-1");
    assert_eq!(rows[0]["name"], "600519 贵州茅台");
    assert_eq!(rows[0]["state"], "succeeded", "`completed` → 中心 `succeeded`");
    assert_eq!(rows[0]["trigger"], "manual", "记录表无触发来源列 ⇒ 一律 manual");
    assert_eq!(rows[0]["startedAt"], 1_700_000_000_000_i64);
    assert_eq!(
        rows[0]["finishedAt"], 1_700_000_050_000_i64,
        "结束时刻只取 completedAt（updatedAt 被回写顶掉，不得用它倒推）：{}",
        rows[0]
    );
    // 缺省项是「不出现」而不是 null（宿主按字段在不在决定耗时列与可否打开）。
    for absent in ["durationMs", "deepLink"] {
        assert!(rows[0].get(absent).is_none(), "缺省字段 `{absent}` 不得进帧：{}", rows[0]);
    }
    assert_eq!(rows[0]["raw"]["analysisDate"], "2026-11-05", "记录视图原样留在 raw");

    // 缺省形态：不传 limit / offset ⇒ 两键都不带（缺省档由宿主取，插件不另抄一份）。
    worker.send(
        &CommandFrame::new(PLUGIN_ID, "list_stock_analysis_records", serde_json::json!({}), 5_000)
            .to_frame_request()
            .expect("命令帧应能编成请求")
            .with_id("req-analysis-rows-2"),
    );
    let second = worker.recv_request();
    assert_eq!(second.args["seam"], "analysis.record");
    assert!(
        second.args["args"].get("limit").is_none() && second.args["args"].get("offset").is_none(),
        "未指定分页 ⇒ 原样不带：{:?}",
        second.args
    );
    // 接缝报错 ⇒ 命令失败并原样透传错误码（空数组会在任务中心伪装成「今天没活」）。
    worker.send_response(
        &FrameResponse::error("TEST_ANALYSIS_RECORD_UNAVAILABLE", "宿主 provider 未注册")
            .with_id(second.request_id.clone().expect("出站请求必须带 request_id")),
    );
    let failed = worker.recv_response();
    assert_eq!(failed.kind, ResponseKind::Error);
    assert_eq!(failed.code.as_deref(), Some("TEST_ANALYSIS_RECORD_UNAVAILABLE"));

    worker.send(&FrameRequest::new(ops::SHUTDOWN, serde_json::json!({})));
    assert_eq!(worker.recv_response().kind, ResponseKind::Success);
    worker.close_stdin();
    assert!(worker.child.wait().expect("等待 worker 退出").success());
}
