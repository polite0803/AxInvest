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
/// 插件贡献的命名命令（manifest `commands[].name` / `taskSources[].command` 必须逐字一致）。
const COMMAND_ACTIVE_RUNS: &str = "list_active_workflow_runs";
/// 一次任务源取数的默认上限：与宿主 `list_active` 的缺省值一致，
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
        // 任务源取数：行数据来自宿主 `workflow.run` 的 `list_active`，插件只做状态词表翻译
        // 与形状收敛（渲染与 run id 拼装归宿主泛型适配器，见 PLAN §97.4）。
        COMMAND_ACTIVE_RUNS => {
            let rows = active_workflow_runs(&frame.input, &request.call_chain, peer);
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
            calls: vec![MARKET_SEAM.to_string(), WORKFLOW_RUN_SEAM.to_string()],
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
/// 数据一律经宿主 `workflow.run` 的 `list_active` op —— 执行态在宿主引擎里，插件不复制
/// 一份登记表（禁区 12）。本函数只干两件宿主干不了/不该干的事：
/// 1. **状态词表翻译**：引擎六态（`completed` / `partially_completed` / `cancelled` 英式拼法）
///    → 任务中心七态（`succeeded` / `canceled` 美式拼法）。表外值一律落 `waiting`，
///    **不猜**成某个具体终态（与宿主 `mapRunTrigger` 同一条纪律）。
/// 2. **形状收敛**：只交宿主泛型适配器认得的字段，`raw` 原样带上引擎视图供详情抽屉。
///
/// ⚠ `partially_completed` 归 `succeeded`：它是终态且**确有产出**，判成 `failed` 会把
/// 「跑完但部分节点没成」的批次误标成失败；细分差异留在 `raw.status` 里，宿主不改口径。
fn active_workflow_runs(
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
            "list_active",
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
/// ⚠ **不交 `finishedAt`**：`list_active` 只含 running / paused 执行（宿主侧过滤见
/// `workflow_run_service.rs:697`），`updatedAt` 是「最近一次推进」而非结束时刻 ⇒ 拿它当
/// 结束时间是编读数。耗时因此留空，由宿主按「未知」不显示。
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
        finished_at: None,
        duration_ms: None,
        deep_link: None,
        raw: Some(execution.clone()),
    };
    serde_json::to_value(row).expect("任务源行记录是可序列化的纯数据")
}

/// 引擎六态 → 任务中心七态（词汇表 = `PLUGIN_TASK_RUN_STATES`，表外一律 `waiting`）。
///
/// ⚠ 今天 `list_active` 只吐 running / paused，终态那三条分支是**防御性**的：
/// 宿主一旦把「近期已结束的批次」纳入同一接缝（B8-2 的候选口径），这里不必改也能翻对；
/// 在那之前不要在 PLAN 里把它当成实测到达过的路径。
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
    use super::{map_execution_state_to_run_state, task_run_row_from_execution};
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
}
