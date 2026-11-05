// SPDX-License-Identifier: AGPL-3.0-only
//! 股票域插件 worker（B4 骨架 + 首个真实能力）。
//!
//! 宿主以 `worker.program` 相对路径 spawn 本进程，双方在 stdin/stdout 上交换长度前缀
//! JSON 帧（`axagent-plugin-proto`）。诊断信息一律走 stderr —— stdout 是协议通道，
//! 混入任何非帧字节都会让宿主读帧失败。
//!
//! **首个能力（B4-c）**：`tool.set` 接缝上的只读工具 `get_stock_quote`。
//! 它是「宿主只读、不持 AppState、不碰凭证」那一档的最小实证：
//! - 工具本身不查库、不读文件、不取时钟；
//! - 行情一律经宿主接缝 `market.quote` 的 `get_quote` op 取（凭证与 provider 都在宿主侧）；
//! - 宿主侧 `RemoteToolSet::load` 会在注册时先调 `list` 取元数据，`exec` 按名执行。
//!
//! 为什么用 `plugin-proto` 的 [`serve`] 而不是自写读循环：插件一旦能主动调宿主，
//! 同一条管道上就同时挂着「我方请求的响应」与「对端发来的新请求」，朴素读一帧写一帧
//! 必然错配。`Peer` 把对端请求进 `deferred` 队列，等响应的同时不丢帧、不死锁。
//!
//! 为什么是纯可执行包（只有 `src/main.rs`、没有 `src/lib.rs`）：分层门禁规则 4 要求每个
//! 库 crate 至少被一份 `Cargo.toml` 引用，而 worker 由宿主 spawn、没有 Cargo 消费方；
//! 门禁对「纯可执行包」已有豁免先例（`crates/schema-gen`），本 crate 照同一形态。

use std::io::{StdinLock, StdoutLock};
use std::process::ExitCode;

use axagent_plugin_proto::{
    AXAGENT_PLUGIN_PROTO_VERSION, CapabilityDecl, CommandFault, CommandFrame, CommandOutcome,
    CommandReply, FrameRequest, FrameResponse, Peer, PluginDeclaration, PluginTaskRunRow,
    ResponseKind, error_codes, is_plugin_task_run_state, ops, serve,
};
use serde_json::{Value, json};

/// 本插件提供的工具名（与宿主登记的工具名逐字一致）。
const TOOL_NAME: &str = "get_stock_quote";
/// 本插件承接的命名命令（manifest `commands[].name` 必须与它逐字一致）。
const COMMAND_GET_QUOTE: &str = "stock_get_quote";
/// 自然语言意图解析命令：跑 `axagent-stock-core` 的**本体**（不是测试夹具的子集词表）。
const COMMAND_PARSE_INTENT: &str = "parse_analysis_intent";
/// 证据质量驱动权重（B9-1）：纯函数，跑 `analysis-engine` 的权威实现。
///
/// 为什么不在插件里重写一份：权重表/阈值是 LLM 决策链的**数值权威**
/// （`evidence_weight.rs` 的档位与 0.15 阈值都有历史校准记录，见 §86 系列），
/// 复制一份就等于让「插件版」与「宿主历史产物」的算法分叉（禁区 12）。
const COMMAND_EVIDENCE_WEIGHTS: &str = "compute_evidence_weights";
/// `tool.set` 接缝的协议版本；随工具集形状变化时递增。
const TOOL_SET_VERSION: &str = "0.1.0";
/// 工具取数走的宿主接缝：宿主把已有行情能力以入站分派的方式暴露给插件。
const MARKET_SEAM: &str = "market.quote";
/// 任务源取数走的宿主接缝（B8-1b）：工作流执行的运行态只有宿主引擎知道。
const WORKFLOW_RUN_SEAM: &str = "workflow.run";
/// 单股分析记录取数走的宿主接缝（#149 第 1 步）：宿主发起的分析记录在 `stock_analyses` 表里，
/// `workflow.run` 按 owner 隔离看不见。清单声明（`taskSources[]`）留给后续批 —— 本步只把
/// 出站准入（`calls`）与本命令的实现打通。
const ANALYSIS_RECORD_SEAM: &str = "analysis.record";
/// 插件贡献的命名命令（manifest `commands[].name` / `taskSources[].command` 必须逐字一致）。
const COMMAND_RECENT_RUNS: &str = "list_recent_workflow_runs";
/// 单股分析记录取数命令（同上：清单声明留给后续批，本步只保证通道可跑）。
const COMMAND_ANALYSIS_RECORDS: &str = "list_stock_analysis_records";
/// 一次任务源取数的默认上限：与宿主 `list_recent` 的缺省值一致，
/// 前端泛型适配器不传 limit 时也走这一档（不是插件自己拍的第二个数）。
const DEFAULT_TASK_SOURCE_LIMIT: u32 = 50;

fn main() -> ExitCode {
    match serve(handle) {
        // 对端关管道 = 宿主停机，正常退出（见 `Peer::run`：EOF 不是错误）。
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("[axagent-plugin-stock] 帧通道失败：{e}");
            ExitCode::FAILURE
        },
    }
}

fn handle(
    request: &FrameRequest,
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> FrameResponse {
    match request.op.as_str() {
        ops::DESCRIBE => declaration(),
        ops::SHUTDOWN => FrameResponse::success(json!({ "ok": true })),
        // `tool.set` 的发现面：宿主要在注册时一次性取回工具元数据（`Tool::name()` 是同步借用）。
        "list" => FrameResponse::success(tools()),
        // `tool.set` 的执行面：宿主发 `{name, args}`，执行结果放 `content`（字符串）。
        "exec" => exec_tool(&request.args, &request.call_chain, peer),
        // T2 命令面：宿主发 `CommandFrame`（`pluginId` 绑定 + 命令名 + 入参 + 预算）。
        ops::COMMAND => handle_command(request, peer),
        other => FrameResponse::error(
            error_codes::PLUGIN_UNKNOWN_OP,
            format!("股票插件未实现该操作：{other}"),
        ),
    }
}

/// 处理一条命令帧：身份自检 → 按命令名分发。
///
/// 身份自检是**第二道防线**（第一道在宿主 `WorkerInvoker::execute_command` 写帧前）：
/// 帧上的 `plugin_id` 必须等于 spawn 时注入的 `AXAGENT_PLUGIN_ID`，防宿主侧错配。
fn handle_command(
    request: &FrameRequest,
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> FrameResponse {
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

    let outcome = match frame.command.as_str() {
        COMMAND_GET_QUOTE => match fetch_quote(&frame.input, &request.call_chain, peer) {
            Ok((_stock_code, quote)) => {
                CommandOutcome::Reply(CommandReply::for_frame(&frame, quote))
            },
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // 纯函数命令：不查库、不调接缝，跑的是 `axagent-stock-core` 的权威实现。
        COMMAND_PARSE_INTENT => match parse_intent(&frame.input) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_EVIDENCE_WEIGHTS => match evidence_weights(&frame.input) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // 任务源取数：行数据来自宿主 `workflow.run` 的 `list_recent`（在跑 + 已结束），
        // 插件只做状态词表翻译与形状收敛（渲染与 run id 拼装归宿主泛型适配器，见 PLAN §97.4）。
        COMMAND_RECENT_RUNS => {
            let rows = recent_workflow_runs(&frame.input, &request.call_chain, peer);
            match rows {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // 单股分析记录取数：行数据来自宿主 `analysis.record` 的 `list`（宿主发起的记录也在内）。
        // 只做「状态词表翻译 + 形状收敛」，行字段与过滤判据见 `stock_analysis_records`。
        COMMAND_ANALYSIS_RECORDS => {
            let rows = stock_analysis_records(&frame.input, &request.call_chain, peer);
            match rows {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        other => CommandOutcome::Fault(CommandFault::for_frame(
            &frame,
            error_codes::PLUGIN_COMMAND_UNKNOWN,
            format!("股票插件没有命令 `{other}` 的实现"),
        )),
    };
    outcome
        .into_frame_response()
        .unwrap_or_else(|e| FrameResponse::error(error_codes::PLUGIN_COMMAND_FAILED, e.to_string()))
}

/// 握手声明：只声明**真正实现**的东西 —— 空能力会让宿主注册一条调不通的接缝。
///
/// `calls` 是出站准入（宿主 `handle_call_seam` 会校验本插件是否声明过目标接缝），
/// 少写一条 = 运行时被拒。
fn declaration() -> FrameResponse {
    FrameResponse::success(
        serde_json::to_value(PluginDeclaration {
            proto_version: AXAGENT_PLUGIN_PROTO_VERSION,
            subscribe: Vec::new(),
            calls: vec![
                MARKET_SEAM.to_string(),
                WORKFLOW_RUN_SEAM.to_string(),
                ANALYSIS_RECORD_SEAM.to_string(),
            ],
            capabilities: vec![CapabilityDecl {
                seam: "tool.set".to_string(),
                op: "exec".to_string(),
                version: TOOL_SET_VERSION.to_string(),
            }],
        })
        .expect("插件声明是可序列化的纯数据"),
    )
}

/// 工具清单（`tool.set` 的 `list` 回帧）。字段名是宿主 `RemoteToolDescriptor` 的 camelCase 形态。
fn tools() -> Value {
    json!([{
        "name": TOOL_NAME,
        "description": "获取 A 股个股实时行情（价格/涨跌/成交量等），行情由宿主行情接缝提供",
        "inputSchema": {
            "type": "object",
            "properties": {
                "stock_code": { "type": "string", "description": "6 位股票代码，如 600519" }
            },
            "required": ["stock_code"]
        },
        "category": "integration"
    }])
}

/// 执行一个命名工具。未知工具显式报错（不静默成功）；参数缺失按 `SEAM_CALL_INVALID_ARGS` 报。
fn exec_tool(
    args: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> FrameResponse {
    let name = args.get("name").and_then(Value::as_str).unwrap_or_default();
    if name != TOOL_NAME {
        return FrameResponse::error(
            error_codes::PLUGIN_CALL_FAILED,
            format!("股票插件的工具集中没有名为 `{name}` 的工具"),
        );
    }

    let input = args.get("args").cloned().unwrap_or(Value::Null);
    match fetch_quote(&input, call_chain, peer) {
        Ok((stock_code, quote)) => {
            // 工具出参按宿主 `ToolExecReply` 的约定：`content` 是字符串。
            FrameResponse::success(json!({
                "content": serde_json::to_string(&quote).unwrap_or_else(|_| quote.to_string()),
                "metadata": { "stock_code": stock_code }
            }))
        },
        Err((code, message)) => FrameResponse::error(code, message),
    }
}

/// 纯函数命令：自然语言 → 结构化意图（`ParsedIntent` 的 camelCase JSON）。
fn parse_intent(input: &Value) -> Result<Value, (String, String)> {
    let Some(text) = input.get("input").and_then(Value::as_str) else {
        return Err((
            error_codes::PLUGIN_COMMAND_INVALID_ARGS.to_string(),
            "`input` 必须是字符串".to_string(),
        ));
    };
    serde_json::to_value(axagent_stock_core::intent_parser::parse_analysis_intent(text)).map_err(
        |e| {
            (error_codes::PLUGIN_COMMAND_FAILED.to_string(), format!("意图解析结果序列化失败：{e}"))
        },
    )
}

/// 纯函数命令：`EvidenceWeightRequest`（camelCase）→ `EvidenceWeightReport`。
///
/// 入参解码严格：缺字段 / 类型不符报 `PLUGIN_COMMAND_INVALID_ARGS`，**不**用默认值兜底 ——
/// 权重算错的后果是决策方向被改写，静默降级比报错危险得多（同 §86 的 v133 观察）。
fn evidence_weights(input: &Value) -> Result<Value, (String, String)> {
    use axagent_analysis_engine::evidence_weight::{
        EvidenceWeightRequest, compute_evidence_weights,
    };
    let request: EvidenceWeightRequest = serde_json::from_value(input.clone()).map_err(|e| {
        (
            error_codes::PLUGIN_COMMAND_INVALID_ARGS.to_string(),
            format!("`EvidenceWeightRequest` 解码失败：{e}"),
        )
    })?;
    serde_json::to_value(compute_evidence_weights(request)).map_err(|e| {
        (error_codes::PLUGIN_COMMAND_FAILED.to_string(), format!("权重报告序列化失败：{e}"))
    })
}

/// 任务源取数命令：`PluginTaskRunRow[]` 的 JSON 数组（契约见 plugin-proto）。
///
/// 数据一律经宿主 `workflow.run` 的 `list_recent` op（在跑的 + 已结束的）—— 执行态在宿主
/// 引擎与宿主登记表里，插件不复制一份（禁区 12）。本函数只干两件宿主干不了/不该干的事：
/// 1. **状态词表翻译**：引擎六态（`completed` / `partially_completed` / `cancelled` 英式拼法）
///    → 任务中心七态（`succeeded` / `canceled` 美式拼法）。表外值一律落 `waiting`，
///    **不猜**成某个具体终态（与宿主 `mapRunTrigger` 同一条纪律）。
/// 2. **形状收敛**：只交宿主泛型适配器认得的字段，`raw` 原样带上引擎视图供详情抽屉。
///
/// ⚠ `partially_completed` 归 `succeeded`：它是终态且**确有产出**，判成 `failed` 会把
/// 「跑完但部分节点没成」的批次误标成失败；细分差异留在 `raw.status` 里，宿主不改口径。
fn recent_workflow_runs(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    // `limit` 容忍缺省与非数字：非数字按缺省档处理并**说明**，不静默改成别的值。
    let limit = input
        .get("limit")
        .and_then(Value::as_u64)
        .map(|v| v.clamp(1, 200) as u32)
        .unwrap_or(DEFAULT_TASK_SOURCE_LIMIT);
    let workflow_id = input.get("workflowId").and_then(Value::as_str).map(str::trim);

    let reply = peer
        .call_seam(
            WORKFLOW_RUN_SEAM,
            "list_recent",
            json!({ "limit": limit, "workflow_id": workflow_id }),
            call_chain.to_vec(),
        )
        .map_err(|e| {
            (error_codes::PLUGIN_COMMAND_FAILED.to_string(), format!("工作流执行接缝调用失败：{e}"))
        })?;
    match reply.kind {
        ResponseKind::Success => {
            let page = reply.value.unwrap_or(Value::Null);
            let executions =
                page.get("executions").and_then(Value::as_array).cloned().unwrap_or_default();
            let rows: Vec<Value> = executions.iter().map(task_run_row_from_execution).collect();
            Ok(Value::Array(rows))
        },
        ResponseKind::Error => Err((
            reply.code.unwrap_or_else(|| error_codes::PLUGIN_COMMAND_FAILED.to_string()),
            reply.message.unwrap_or_else(|| "工作流接缝返回错误但未说明".to_string()),
        )),
    }
}

/// 引擎执行视图（camelCase JSON）→ 任务中心一行。
///
/// 先收成 [`PluginTaskRunRow`] 再序列化，而不是手搓 `json!` 键名：行形状的唯一权威在
/// plugin-proto，宿主前端按那份 camelCase 消费（禁区 12）—— 手搓键名漂一个字母不会报错，
/// 只会让任务中心静默少一列。
///
/// ⚠ **不交 `deepLink`**：工作流执行详情今天没有独立站内路由
/// （2026-10-28 已删 `/workflow` 一类旧链，见 `ContentArea.tsx:197`）⇒ 造一个指向 404 的
/// 深链比没有深链更坏。宿主泛型适配器据此把该 run 的 `openSource` 能力关成 false。
///
/// `finishedAt` 只透传宿主的 `finishedAt`，**不由 `updatedAt` 倒推**：宿主对已退出的执行
/// 才盖这个戳（在跑的行没这键），拿「最近一次推进」冒充结束时刻会让耗时列变成编读数。
/// 耗时也不在这里算 —— 宿主泛型适配器有两端点就会算（`resolveDurationMs`），插件再算一份
/// 就是第二套口径。
fn task_run_row_from_execution(execution: &Value) -> Value {
    let status = execution.get("status").and_then(Value::as_str).unwrap_or_default();
    let execution_id = execution.get("executionId").and_then(Value::as_str).unwrap_or_default();
    let workflow_id = execution.get("workflowId").and_then(Value::as_str).unwrap_or_default();
    let state = map_execution_state_to_run_state(status);
    debug_assert!(
        is_plugin_task_run_state(state),
        "状态映射产出了表外值 `{state}`：宿主会静默按未知处理"
    );
    let row = PluginTaskRunRow {
        native_id: execution_id.to_string(),
        // 同一 workflow 会有多条执行 ⇒ 名字必须能区分开，不能只显示 workflow id。
        name: format!("{workflow_id} · {execution_id}"),
        state: state.to_string(),
        // 工作流执行由插件受理，中心一律标 manual：引擎侧没有「定时/系统发起」的列，
        // 猜成 schedule 就是把不知道伪装成读数。
        trigger: Some("manual".to_string()),
        started_at: execution.get("createdAt").and_then(Value::as_i64),
        finished_at: execution.get("finishedAt").and_then(Value::as_i64),
        duration_ms: None,
        deep_link: None,
        raw: Some(execution.clone()),
    };
    serde_json::to_value(row).expect("任务源行记录是可序列化的纯数据")
}

/// 引擎六态 → 任务中心七态（词汇表 = `PLUGIN_TASK_RUN_STATES`，表外一律 `waiting`）。
///
/// 终态那三条分支今天**实测可达**：宿主 `list_recent` 会把已退出的执行放进同一页
/// （#150；此前 `list_active` 只吐 running / paused，那三条只是防御性代码）。
/// 表外值仍落 `waiting` —— 引擎将来加第七态时这里不会误判成某个具体终态。
fn map_execution_state_to_run_state(status: &str) -> &'static str {
    match status {
        "running" => "running",
        "paused" => "paused",
        "completed" | "partially_completed" => "succeeded",
        "failed" => "failed",
        "cancelled" => "canceled",
        _ => "waiting",
    }
}

/// 单股分析记录取数命令：`PluginTaskRunRow[]` 的 JSON 数组。
///
/// 数据一律经宿主 `analysis.record` 的 `list` 取 —— 记录表与「宿主发起的记录」只有宿主知道
/// （`workflow.run` 按 owner 隔离，`list_recent` 一条都看不见宿主发起的分析，判据见
/// `harness/src/analysis_record.rs` 模块头）。本函数只做宿主干不了的两件事：
///
/// 1. **状态词表翻译**：宿主落库值（`completed` / `failed` / `error` / `interrupted` /
///    `canceled` / `cancelled` / `paused` / `queued` / `pending` / `running`）→ 任务中心七态。
///    这张表此前住在宿主适配器 `mapAnalysisState`（`stockAnalysis.adapter.ts`），#149 第 2 步
///    删适配器后**以这里为准**；表外值落 `failed`（沿用宿主适配器的裁定：不得默认成
///    `running`，否则僵死记录会一直显示「运行中」并触发轮询）。
/// 2. **根记录收窄**：`parentAnalysisId != null` 的是时间旅行批次的子行（宿主适配器同样只取
///    根记录，replay 子行由 time-travel 源代表）。⚠ 收窄发生在插件侧 ⇒ 宿主返回的 `total`
///    仍是整表行数，**不能**当作「根记录总数」参与分页算术（接缝口径就是这样，见契约模块头）。
///
/// ⚠ **不交 `deepLink`**：站内路由是宿主的路由契约（`src/lib/investRoutes.ts`），插件侧
/// 抄一份就会随着宿主改路由静默 404。缺省 ⇒ 该 run 暂不可打开；路由拼装归后续批（与
/// `task_run_row_from_execution` 的选择同一条判据）。
fn stock_analysis_records(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    // `limit` / `offset` 原样透传：**不在插件侧钳位**（越界由宿主按契约区间拒收），
    // 缺省也交给宿主 —— 插件再抄一份 50 / 200 就是第二套口径。
    let mut args = serde_json::Map::new();
    if let Some(limit) = input.get("limit").filter(|v| !v.is_null()) {
        args.insert("limit".to_string(), limit.clone());
    }
    if let Some(offset) = input.get("offset").filter(|v| !v.is_null()) {
        args.insert("offset".to_string(), offset.clone());
    }

    let reply = peer
        .call_seam(ANALYSIS_RECORD_SEAM, "list", Value::Object(args), call_chain.to_vec())
        .map_err(|e| {
            (error_codes::PLUGIN_COMMAND_FAILED.to_string(), format!("分析记录接缝调用失败：{e}"))
        })?;
    match reply.kind {
        ResponseKind::Success => {
            let page = reply.value.unwrap_or(Value::Null);
            let records =
                page.get("records").and_then(Value::as_array).cloned().unwrap_or_default();
            let rows: Vec<Value> = records
                .iter()
                .filter(|record| match record.get("parentAnalysisId") {
                    // 只交根记录：`null` / 缺键都算根（接缝对 `None` 的线格式是 `null`）。
                    None | Some(Value::Null) => true,
                    Some(_) => false,
                })
                .map(task_run_row_from_analysis_record)
                .collect();
            Ok(Value::Array(rows))
        },
        // 接缝报错 ⇒ 命令失败并**原样透传错误码**，不得退化成空列表
        // （空数组在任务中心看起来像「今天没活」）。
        ResponseKind::Error => Err((
            reply.code.unwrap_or_else(|| error_codes::PLUGIN_COMMAND_FAILED.to_string()),
            reply.message.unwrap_or_else(|| "分析记录接缝返回错误但未说明".to_string()),
        )),
    }
}

/// 分析记录视图（camelCase JSON）→ 任务中心一行。
///
/// 收成 [`PluginTaskRunRow`] 再序列化（行形状权威在 plugin-proto，不手搓键名）。
///
/// 结束时刻只用**完成时刻** `completedAt`，且仅在终态（succeeded / failed）且
/// `completedAt >= createdAt` 时透出 —— 宿主 `updatedAt` 会被交易意图审核 / 结果回写顶掉
/// （现网 65% 的 completed 行漂移 ≥ 24 小时），拿它算耗时会显示「跑了好几天」。耗时不在
/// 这里算：两端点相减只发生在宿主泛型适配器一处。
fn task_run_row_from_analysis_record(record: &Value) -> Value {
    let status = record.get("status").and_then(Value::as_str).unwrap_or_default();
    let state = map_analysis_state_to_run_state(status);
    debug_assert!(
        is_plugin_task_run_state(state),
        "状态映射产出了表外值 `{state}`：宿主会静默按未知处理"
    );
    let id = record.get("id").and_then(Value::as_str).unwrap_or_default();
    let stock_code = record.get("stockCode").and_then(Value::as_str).unwrap_or_default();
    let stock_name = record.get("stockName").and_then(Value::as_str).unwrap_or_default();
    let created_at = record.get("createdAt").and_then(Value::as_i64);
    let completed_at = record.get("completedAt").and_then(Value::as_i64);
    let finished_at = match (state, created_at, completed_at) {
        ("succeeded" | "failed", Some(started), Some(completed)) if completed >= started => {
            Some(completed)
        },
        _ => None,
    };
    let row = PluginTaskRunRow {
        native_id: id.to_string(),
        // 不含 i18n 前缀：宿主适配器把 `tasksCenter.source.stockAnalysis` 拼在名字前面，
        // 插件侧不持有那套译键（`PluginTaskRunRow::name` 的「不经宿主 i18n」就指这一格）。
        name: format!("{stock_code} {stock_name}").trim().to_string(),
        state: state.to_string(),
        // 记录表没有触发来源列（`config_id` 现网 0 行有值）⇒ 定时发起的分析无法与手动区分。
        // 一律标 manual，与宿主适配器同口径（`stockAnalysis.adapter.ts:55` 的注释）。
        trigger: Some("manual".to_string()),
        started_at: created_at,
        finished_at,
        duration_ms: None,
        deep_link: None,
        // 详情投影：本插件 `ui.json` 的 `stock-analysis-detail` 用 Table 节点 `id = "rows"`，
        // 宿主泛型适配器把 `run.raw` 原样当 `dataContext` 交给 `PluginSchemaCard`
        // （`pluginTaskSource.adapter.ts` 的 `dataContext: run.raw`）⇒ 键名必须与节点 id 对齐。
        // 记录本身**一个字段不减**地落在 `rows[0]` 里：包一层是为了对上 schema 的数组入口，
        // 不是重新裁剪字段（裁剪等于在插件里造第二套 DTO）。
        raw: Some(json!({ "rows": [record.clone()] })),
    };
    serde_json::to_value(row).expect("任务源行记录是可序列化的纯数据")
}

/// 分析记录落库状态 → 任务中心七态（词汇表 = `PLUGIN_TASK_RUN_STATES`）。
///
/// 逐条对齐宿主适配器的 `mapAnalysisState`（`stockAnalysis.adapter.ts`，大小写不敏感：宿主列
/// 是自由文本）。表外值落 `failed` —— 这不是「已知失败」，而是宿主适配器当时的裁定：
/// **不得默认成 `running`**（那会让僵死记录继续显示「运行中」并触发轮询）。#149 第 2 步删
/// 适配器后本表即唯一权威，改默认档必须同时改这一格的理由。
fn map_analysis_state_to_run_state(status: &str) -> &'static str {
    match status.to_ascii_lowercase().as_str() {
        "running" => "running",
        "paused" => "paused",
        "completed" => "succeeded",
        "failed" | "error" => "failed",
        "interrupted" | "canceled" | "cancelled" => "canceled",
        "queued" | "pending" => "queued",
        // 未知状态不得默认成 running（会让僵死记录一直显示运行中并触发轮询）。
        _ => "failed",
    }
}

/// 取一条个股行情，返回 `(入参里的股票代码, 行情 JSON)`。
///
/// 工具执行面与命令面共用的唯一实现（不写第二遍取数逻辑）。出站走宿主接缝：
/// 凭证 / provider / 限流都在宿主侧，插件只传代码；`call_chain` 必须原样带上入站链，
/// 否则宿主侧的环检测收不到这条边。
fn fetch_quote(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<(String, Value), (String, String)> {
    let Some(stock_code) =
        input.get("stock_code").and_then(Value::as_str).map(str::trim).filter(|c| !c.is_empty())
    else {
        return Err((
            error_codes::SEAM_CALL_INVALID_ARGS.to_string(),
            "`stock_code` 缺失或为空白".to_string(),
        ));
    };

    let reply = peer
        .call_seam(
            MARKET_SEAM,
            "get_quote",
            json!({ "stock_code": stock_code }),
            call_chain.to_vec(),
        )
        .map_err(|e| {
            (error_codes::PLUGIN_CALL_FAILED.to_string(), format!("行情接缝调用失败：{e}"))
        })?;

    match reply.kind {
        ResponseKind::Success => Ok((stock_code.to_string(), reply.value.unwrap_or(Value::Null))),
        ResponseKind::Error => Err((
            reply.code.unwrap_or_else(|| error_codes::PLUGIN_CALL_FAILED.to_string()),
            reply.message.unwrap_or_else(|| "行情接缝返回了错误但没有说明".to_string()),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        map_analysis_state_to_run_state, map_execution_state_to_run_state,
        task_run_row_from_analysis_record, task_run_row_from_execution,
    };
    use axagent_plugin_proto::{PLUGIN_TASK_RUN_STATES, is_plugin_task_run_state};
    use serde_json::{Value, json};

    /// 引擎视图 → 行记录：键必须 camelCase，缺省端点必须**不出现**（不是 `null`）。
    ///
    /// 钉这条是因为宿主泛型适配器按「字段在不在」决定要不要显示耗时/是否可打开；
    /// `null` 与「缺省」在 `Option` 反序列化上是两种形状，手搓 `json!` 极易写错。
    #[test]
    fn task_run_row_uses_camel_case_and_omits_absent_endpoints() {
        let execution = json!({
            "executionId": "exec-7",
            "workflowId": "wf-daily",
            "status": "running",
            "currentNodeId": "node-2",
            "ownerPluginId": "stock",
            "createdAt": 1_700_000_000_000_i64,
            "updatedAt": 1_700_000_005_000_i64,
        });
        let row = task_run_row_from_execution(&execution);

        assert_eq!(row["nativeId"], "exec-7");
        assert_eq!(row["name"], "wf-daily · exec-7", "同一 workflow 多条执行 ⇒ 名字必须可区分");
        assert_eq!(row["state"], "running");
        assert_eq!(row["trigger"], "manual");
        assert_eq!(row["startedAt"], 1_700_000_000_000_i64);
        assert_eq!(row["raw"]["currentNodeId"], "node-2", "引擎视图原样留在 raw 供详情");
        for absent in ["started_at", "finishedAt", "durationMs", "deepLink"] {
            // `started_at`（蛇形）在列是专门盯 `rename_all = "camelCase"` 漏掉的情形。
            assert!(row.get(absent).is_none(), "缺省字段 / 蛇形键 `{absent}` 不得进帧：{row}");
        }
    }

    /// 终态行：宿主的 `finishedAt` 必须原样透出（耗时列的唯一依据），但仍不交 `durationMs`。
    ///
    /// 这一条是 #150 的正向断言 —— `list_recent` 之前插件永远拿不到 `finishedAt`，
    /// 「不出现」是当时的正确形状；现在它出现而 `durationMs` 不出现，才说明两端点算耗时
    /// 这件事只发生在宿主泛型适配器一处（`resolveDurationMs`），插件没造第二套口径。
    #[test]
    fn task_run_row_passes_host_finished_at_but_not_duration() {
        let row = task_run_row_from_execution(&json!({
            "executionId": "exec-0",
            "workflowId": "wf-daily",
            "status": "completed",
            "createdAt": 1_699_999_990_000_i64,
            "updatedAt": 1_699_999_999_000_i64,
            "finishedAt": 1_699_999_999_000_i64,
        }));
        assert_eq!(row["state"], "succeeded");
        assert_eq!(row["finishedAt"], 1_699_999_999_000_i64);
        assert!(row.get("durationMs").is_none(), "耗时由宿主按两端点算：{row}");
        assert!(row.get("deepLink").is_none(), "没有站内路由就不交深链：{row}");

        // 在跑行没有该键 ⇒ 不能凭 `updatedAt` 造一个结束时刻
        let running = task_run_row_from_execution(&json!({
            "executionId": "exec-1",
            "workflowId": "wf-daily",
            "status": "running",
            "createdAt": 1_700_000_000_000_i64,
            "updatedAt": 1_700_000_005_000_i64,
        }));
        assert!(running.get("finishedAt").is_none(), "在跑行的耗时是未知，不是 0：{running}");
    }

    /// `createdAt` 缺失 ⇒ `startedAt` 缺省而非 `null`；宿主按「未知」不显示，不编一个 0 点。
    #[test]
    fn task_run_row_drops_started_at_when_engine_view_lacks_created_at() {
        let row = task_run_row_from_execution(&json!({ "executionId": "e", "workflowId": "w" }));
        assert!(row.get("startedAt").is_none(), "缺 createdAt 应产出缺省而非 null：{row}");
        // 表外状态（这里是「没有 status」）落 waiting，不猜成 running。
        assert_eq!(row["state"], "waiting");
        assert_eq!(row["nativeId"], "e", "有 id 时照常透出，不因缺其它字段而丢主键");
    }

    /// 状态翻译：六态逐条对齐七态词表，表外值不得被判成合法状态。
    #[test]
    fn execution_status_maps_into_task_center_vocabulary_only() {
        let cases = [
            ("running", "running"),
            ("paused", "paused"),
            ("completed", "succeeded"),
            // 终态且确有产出 ⇒ 判成 succeeded；判 failed 会把「部分节点没成」误标成失败。
            ("partially_completed", "succeeded"),
            ("failed", "failed"),
            // 引擎用英式 cancelled，中心用美式 canceled：拼法差异必须在插件侧翻掉。
            ("cancelled", "canceled"),
        ];
        for (from, to) in cases {
            let mapped = map_execution_state_to_run_state(from);
            assert_eq!(mapped, to, "`{from}` 应翻成 `{to}`");
            assert!(is_plugin_task_run_state(mapped), "映射产出 `{mapped}` 不在词表内");
        }
        // 表外值（含大小写/拼法漂移）一律 waiting，且词表本身没被改动。
        for unknown in ["", "Running", "canceled ", "created", "unknown"] {
            assert_eq!(map_execution_state_to_run_state(unknown), "waiting");
        }
        assert_eq!(PLUGIN_TASK_RUN_STATES.len(), 7);
    }

    /// 行记录必须是宿主可 `JSON.parse` 的纯数组元素：本测试锁住「命令返回数组」这一形状。
    #[test]
    fn task_run_rows_are_plain_json_objects_for_frontend_parsing() {
        let rows: Vec<Value> =
            [json!({ "executionId": "a", "workflowId": "w", "status": "paused" })]
                .iter()
                .map(task_run_row_from_execution)
                .collect();
        let text = serde_json::to_string(&rows).expect("行数组应可序列化");
        let back: Vec<Value> = serde_json::from_str(&text).expect("宿主 JSON.parse 读得回");
        assert_eq!(back[0]["state"], "paused");
        assert!(back[0].is_object(), "元素必须是对象，宿主泛型适配器按字段取值");
    }

    /// 单股分析记录的状态翻译（#149 第 3 步：宿主 `stockAnalysis.adapter.ts` 删除后，
    /// 这张表是唯一权威 ⇒ 原测试断言搬到这里，不再留在前端）。
    ///
    /// 覆盖点逐条来自被删的 `mapAnalysisState`：大小写不敏感（宿主列是自由文本）、
    /// 七态对齐、以及**表外值落 failed 而不是 running**（默认成 running 会让僵死记录
    /// 一直显示「运行中」并持续轮询）。
    #[test]
    fn analysis_status_maps_into_task_center_vocabulary() {
        let cases = [
            ("running", "running"),
            ("RUNNING", "running"),
            ("completed", "succeeded"),
            ("failed", "failed"),
            ("error", "failed"),
            ("interrupted", "canceled"),
            ("canceled", "canceled"),
            ("cancelled", "canceled"),
            ("paused", "paused"),
            ("queued", "queued"),
            ("pending", "queued"),
            // 表外值：不得默认成 running
            ("weird", "failed"),
            ("", "failed"),
        ];
        for (from, to) in cases {
            let mapped = map_analysis_state_to_run_state(from);
            assert_eq!(mapped, to, "`{from}` 应翻成 `{to}`");
            assert!(is_plugin_task_run_state(mapped), "映射产出 `{mapped}` 不在七态词表内");
        }
    }

    /// 结束时刻只认 `completedAt`，且只在终态透出；`updatedAt` 是编辑时刻，不得冒充完成时刻。
    ///
    /// 现网 65% 的 completed 行 `updatedAt` 被交易意图审核 / 结果回写顶掉（可达数天），
    /// 拿它算耗时会显示「跑了好几天」——这条断言就是那个坑的钉子。
    #[test]
    fn analysis_row_uses_completed_at_only_and_drops_updated_at() {
        let completed = task_run_row_from_analysis_record(&json!({
            "id": "a1",
            "stockCode": "300308",
            "stockName": "中际旭创",
            "status": "completed",
            "createdAt": 1_000_i64,
            "updatedAt": 4_000_i64,
            "completedAt": 4_000_i64,
            "parentAnalysisId": null,
        }));
        assert_eq!(completed["state"], "succeeded");
        assert_eq!(completed["startedAt"], 1_000_i64);
        assert_eq!(completed["finishedAt"], 4_000_i64);

        // completed 但 completedAt 缺失 ⇒ 结束时刻未知（不是拿 updatedAt 顶）
        let no_completed_at = task_run_row_from_analysis_record(&json!({
            "id": "a2",
            "stockCode": "300308",
            "stockName": "中际旭创",
            "status": "completed",
            "createdAt": 1_000_i64,
            "updatedAt": 9_999_999_i64,
            "completedAt": null,
        }));
        assert!(no_completed_at.get("finishedAt").is_none(), "不得用 updatedAt 冒充：{no_completed_at}");

        // interrupted：算 canceled，且回收时刻不是完成时刻
        let interrupted = task_run_row_from_analysis_record(&json!({
            "id": "a3",
            "status": "interrupted",
            "createdAt": 1_000_i64,
            "updatedAt": 9_999_999_i64,
        }));
        assert_eq!(interrupted["state"], "canceled");
        assert!(interrupted.get("finishedAt").is_none(), "回收时刻不得当完成时刻：{interrupted}");

        // 在跑行：只有开始时间
        let running = task_run_row_from_analysis_record(&json!({
            "id": "a4",
            "status": "running",
            "createdAt": 1_000_i64,
        }));
        assert_eq!(running["state"], "running");
        assert_eq!(running["startedAt"], 1_000_i64);
        assert!(running.get("finishedAt").is_none());
    }

    /// 行形状：名字不带宿主 i18n 前缀、触发来源一律 manual、不交 `deepLink`，
    /// 记录按详情 schema 的节点 id 收进 `raw.rows`（`stock-analysis-detail` 的 Table）。
    ///
    /// 后端 adapter 删除后，「打开」按钮随 `deepLink` 一起消失 —— 这条**正向**钉住
    /// 「不交深链」是当前裁定，而不是漏字段（改裁定要同时改这里与 PLAN §101.5-2）。
    #[test]
    fn analysis_row_shape_and_detail_projection() {
        let record = json!({
            "id": "a5",
            "stockCode": "300308",
            "stockName": "中际旭创",
            "analysisDate": "2026-11-05",
            "status": "running",
            "createdAt": 1_000_i64,
            "updatedAt": 2_000_i64,
            "completedAt": null,
            "analysisKind": "live",
            "parentAnalysisId": null,
        });
        let row = task_run_row_from_analysis_record(&record);

        assert_eq!(row["nativeId"], "a5");
        assert_eq!(row["name"], "300308 中际旭创", "名字由插件拼，不带宿主 i18n 前缀");
        assert_eq!(row["trigger"], "manual", "记录表无触发来源列 ⇒ 与宿主原 adapter 同口径");
        assert!(row.get("deepLink").is_none(), "插件不交深链：{row}");
        assert!(row.get("durationMs").is_none(), "耗时只由宿主按两端点算：{row}");

        let rows = row["raw"]["rows"].as_array().expect("详情投影必须是 rows 数组");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["analysisDate"], "2026-11-05", "记录字段原样保留，不裁剪");
        assert_eq!(rows[0]["analysisKind"], "live");
        assert_eq!(rows[0], record, "投影只包一层数组，不改任何键");
    }
}
