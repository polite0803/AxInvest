#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * OPC 域 worker 源码生成器（单一模板源 + 强校验）。
 *
 * 为什么存在：11 个声明式域要翻成 worker，每域都要一份 Rust 二进制、协议测试、页面 TS
 * 和 bundle 校验器。手写复制会在第 3 个域开始漂移（命令名漏一条、域 id 写错、接缝少声明），
 * 漂移后各仓 CI 仍各自绿 —— 与「数量对账代替逐项对账」是同一类失效。
 *
 * 权威来源：scripts/lib/plugin-contract.json 的 commands[]（命令面）与 domainPackId；
 * 唯一模板：本脚本内的固定骨架（命令→工具映射表由契约派生），不由某个域的旧文件反向推导。
 * game_dev 也由本脚本生成，--check 会把它与模板对账，模板改动必须逐域同步。
 *
 * 生成范围：Rust crate 源 + 页面 TS 源 + 本地校验器 + vendor SDK 副本。
 * 不生成 web/pages/domain-data.js（由 generate-declarative-opc-pages.mjs 从 YAML 生成）
 * 与 web/pages/bundle.js（由 esbuild 从 src-web/main.ts 构建）。
 *
 * 用法：
 *   node scripts/generate-opc-worker-source.mjs --domain ai_research
 *   node scripts/generate-opc-worker-source.mjs --domain ai_research --check
 */
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const argv = process.argv.slice(2);
const CHECK = argv.includes("--check");
const domain = argv[argv.indexOf("--domain") + 1];
const fail = (message) => {
  console.error(`FAIL: ${message}`);
  process.exit(1);
};

if (!domain || domain.startsWith("--")) fail("需要 --domain <域包 id>");
const CONTRACT = JSON.parse(readFileSync(join(ROOT, "scripts", "lib", "plugin-contract.json"), "utf8"));
const entry = CONTRACT.plugins.find((p) => p.domainPackId === domain);
if (!entry) fail(`契约缺少域 ${domain}`);
if (entry.strategy !== "worker") fail(`域 ${domain} 尚未翻牌为 worker（契约 strategy=${entry.strategy}）`);
if (entry.manifestFactory !== "opcWorkerDomainManifest") {
  fail(`域 ${domain} 的 manifestFactory 应为 opcWorkerDomainManifest`);
}

const plugin = entry.name;
const binName = `axagent-plugin-${plugin}`;
const targetDir = join(ROOT, "axagent-plugins", "crates", `plugin-${plugin}`);
const templateDir = join(ROOT, "axagent-plugins", "crates", "plugin-opc-game-dev", "vendor", "sdk");
const runCommand = entry.commands.find((command) => command.endsWith("_run_automation_rules"));
if (!runCommand) fail(`域 ${domain} 缺少自动化规则命令`);
const prefix = `opc_${domain}_`;
// 特殊域的 `batch_validate_entities` 走 opc.store 读缝，不是宿主工具；从契约命令名
// 后缀派生，避免为每个域复制一份「特殊命令清单」。其余命令统一走 tool.set/exec。
const storeCommands = entry.commands.filter((command) => command.endsWith("_batch_validate_entities"));
const toolCommands = entry.commands
  .filter((command) => command !== runCommand && !storeCommands.includes(command))
  .map((command) => {
    if (!command.startsWith(prefix)) fail(`命令 ${command} 不以 ${prefix} 开头`);
    const suffix = command.slice(prefix.length);
    const tool = suffix.split("_").map((part) => part[0].toUpperCase() + part.slice(1)).join("");
    return { command, tool };
  });
if (toolCommands.length === 0) fail(`域 ${domain} 没有工具型命令`);
if (storeCommands.length > 1) fail(`域 ${domain} 声明了多条批量校验命令，生成器只支持一条`);

/** 通用命令入参样例：只给常见工具一份可读样例，其余工具由宿主校验真实 schema。 */
function sample(tool, lang) {
  const value = tool === "WebSearch" ? { query: "搜索关键词" }
    : tool === "FileRead" ? { path: "README.md" }
      : tool === "FileWrite" ? { path: "note.md", content: "正文" }
        : tool === "BatchValidateEntities" ? { entities: [{ entityType: domain, data: {} }] }
          : {};
  const json = JSON.stringify(value, null, 2)
    .split("\n")
    .map((line, index) => (index === 0 ? line : `        ${line}`))
    .join("\n");
  return lang === "ts" ? JSON.stringify(value) : json;
}

function cargoToml() {
  return `# SPDX-License-Identifier: AGPL-3.0-only
[package]
name = "${binName}"
version.workspace = true
edition.workspace = true
license.workspace = true
publish = false
description = "OPC ${domain} 域包 worker：域专属命令经宿主接缝复用工具实现"

[[bin]]
name = "${binName}"
path = "src/main.rs"

[dependencies]
axagent-plugin-opc-core = { version = "3.0.0", path = "../plugin-opc-core" }
axagent-plugin-proto = { version = "3.0.0", path = "../../../axagent-core/crates/plugin-proto" }
serde_json = "1"

[lints]
workspace = true

[dev-dependencies]
axagent-plugin-testkit = { version = "3.0.0", path = "../plugin-testkit" }
`;
}

function mainRs() {
  const rows = toolCommands
    .map(({ command, tool }) => `    ("${command}", "${tool}"),`)
    .join("\n");
  return `// SPDX-License-Identifier: AGPL-3.0-only
//! ${domain} 域包 worker：${entry.commands.length} 条域专属命令。
//!
//! 命令实现只做「形状校验 + 固定域 id + 转发接缝」，工具逻辑留在宿主：
//! 每条工具命令走 \`tool.set/exec\`，自动化规则与实体批量校验走 \`opc.store\`。
//! 这是可执行 Rust 二进制，dist 中产出 worker 产物与 \`worker://\` 命令。
//! 命令→工具映射由 scripts/generate-opc-worker-source.mjs 从统一契约派生，勿手工改本文件。

use std::io::{StdinLock, StdoutLock};
use std::process::ExitCode;

use axagent_plugin_opc_core as core;
use axagent_plugin_proto::{
    AXAGENT_PLUGIN_PROTO_VERSION, CommandFault, CommandFrame, CommandOutcome, CommandReply,
    FrameRequest, FrameResponse, Peer, PluginDeclaration, error_codes, ops, serve,
};
use serde_json::{Value, json};

type WorkerPeer = Peer<StdinLock<'static>, StdoutLock<'static>>;
const PLUGIN_NAME: &str = "${plugin}";
/// 固定注入的域包 id：调用方不能通过命令入参越域触达别的域包规则。
const DOMAIN_ID: &str = "${domain}";
const CMD_RUN_RULES: &str = "${runCommand}";
${storeCommands.map((command) => `/// 实体批量校验走 opc.store 读缝，域 id 由 worker 固定注入。
const CMD_BATCH_VALIDATE: &str = "${command}";`).join("\n")}
/// 域命令 → 宿主工具名。入参逐字透传，工具入参的真实 schema 由宿主负责校验。
const TOOL_COMMANDS: &[(&str, &str)] = &[
${rows}
];

fn own_id_belongs_to_this_plugin(own_id: &str) -> bool {
    own_id == PLUGIN_NAME || own_id.starts_with(&format!("{PLUGIN_NAME}@"))
}

fn declaration() -> FrameResponse {
    FrameResponse::success(
        serde_json::to_value(PluginDeclaration {
            proto_version: AXAGENT_PLUGIN_PROTO_VERSION,
            subscribe: Vec::new(),
            // 两条出站接缝都要声明：少一条，宿主在 \`call_seam\` 入口即拒绝（worker.rs 的
            // \`declared_calls\` 闸），错误发生在取 provider 之前。
            calls: vec![core::OPC_STORE_SEAM.to_string(), core::TOOL_SET_SEAM.to_string()],
            capabilities: Vec::new(),
        })
        .expect("插件声明是可序列化的纯数据"),
    )
}

/// 工具型命令：入参是对象才能逐字作为工具 args 透传，形状闸在命令层完成。
fn call_tool(
    frame: &CommandFrame,
    call_chain: &[String],
    peer: &mut WorkerPeer,
    op: &str,
    tool_name: &str,
) -> Result<Value, (String, String)> {
    let args = core::require_object_input(&frame.input, op)?;
    core::call_builtin_tool(peer, tool_name, args, call_chain)
}

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
    if frame.plugin_id != own_id || !own_id_belongs_to_this_plugin(&own_id) {
        return CommandFault::for_frame(
            &frame,
            error_codes::PLUGIN_COMMAND_ID_MISMATCH,
            format!("帧/身份与 \`{PLUGIN_NAME}\` 不符"),
        )
        .into_frame_response();
    }
    let outcome = if frame.command == CMD_RUN_RULES {
        core::run_domain_automation_rules(DOMAIN_ID, &frame.input, &request.call_chain, peer)
    }${storeCommands.length ? ` else if frame.command == CMD_BATCH_VALIDATE {
        core::run_domain_batch_validate_entities(DOMAIN_ID, &frame.input, &request.call_chain, peer)
    }` : ""} else if let Some((name, tool)) = TOOL_COMMANDS
        .iter()
        .find(|(name, _)| *name == frame.command.as_str())
    {
        call_tool(&frame, &request.call_chain, peer, name, tool)
    } else {
        return CommandFault::for_frame(
            &frame,
            error_codes::PLUGIN_COMMAND_UNKNOWN,
            format!("${plugin} 插件没有命令 \`{}\`", frame.command),
        )
        .into_frame_response();
    };
    let outcome = match outcome {
        Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
        Err((code, message)) => {
            CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
        },
    };
    outcome
        .into_frame_response()
        .unwrap_or_else(|e| FrameResponse::error(error_codes::PLUGIN_COMMAND_FAILED, e.to_string()))
}

fn main() -> ExitCode {
    let result = serve(|request: &FrameRequest, peer: &mut WorkerPeer| match request.op.as_str() {
        ops::DESCRIBE => declaration(),
        ops::COMMAND => handle_command(request, peer),
        ops::SHUTDOWN => FrameResponse::success(json!({ "ok": true })),
        other => FrameResponse::error(
            error_codes::PLUGIN_UNKNOWN_OP,
            format!("${plugin} 插件未实现该操作：{other}"),
        ),
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("${plugin} worker 退出：{e}");
            ExitCode::FAILURE
        },
    }
}
`;
}

function protocolRs() {
  const cases = toolCommands
    .map(({ command, tool }) => `        ("${command}", "${tool}", serde_json::json!(${sample(tool, "rust")})),`)
    .join("\n");
  const first = toolCommands[0];
  // 特殊域才生成：批量校验命令走 opc.store，锁固定域 id、接缝 op 与 entities 形状闸。
  const batchTest = storeCommands.length
    ? `
#[test]
fn batch_validation_injects_fixed_domain_id_and_checks_shape() {
    let mut worker = Worker::spawn();
    worker.send(
        &command(
            "${storeCommands[0]}",
            serde_json::json!({
                "capabilityPackId": "security",
                "entities": [{ "entityType": "${domain}", "data": { "amount": 1 } }],
            }),
        )
        .with_id("req-batch"),
    );
    let call = worker.recv_request();
    assert_eq!(call.op, ops::CALL_SEAM);
    assert_eq!(call.args["seam"], "opc.store");
    assert_eq!(call.args["op"], "batch_validate_entities");
    assert_eq!(call.args["args"]["domain_pack_id"], "${domain}", "不得让调用方越域");
    assert_eq!(call.args["args"]["entities"][0]["entityType"], "${domain}");
    assert!(call.args["args"].get("capabilityPackId").is_none());

    worker.send_response(
        &FrameResponse::success(serde_json::json!({ "valid": true }))
            .with_id(call.request_id.clone().expect("出站请求必须带 request_id")),
    );
    assert_eq!(worker.recv_response().kind, ResponseKind::Success);
    worker.expect_clean_exit();

    let mut worker = Worker::spawn();
    worker.send(
        &command("${storeCommands[0]}", serde_json::json!({ "entities": 1 }))
            .with_id("req-batch-shape"),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Error, "entities 非数组必须在命令层被拒");
    assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_COMMAND_INVALID_ARGS));
    worker.expect_clean_exit();
}

`
    : "";
  return `// SPDX-License-Identifier: AGPL-3.0-only
//! ${domain} worker 的真进程 + 真管道协议测试。
//!
//! 锁三件事：① 声明的两条出站接缝（少一条宿主即拒）；② 每条域命令转发到正确的
//! seam / op，且入参形状不漂移；③ 自动化规则命令固定注入 \`${domain}\` 域 id，调用方
//! 不能越域。宿主工具逻辑 / 数据主权都不在 worker 内，故这里只验帧面。

use std::io::{BufReader, BufWriter};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use axagent_plugin_proto::{
    CommandFrame, FrameRequest, FrameResponse, ResponseKind, error_codes, ops, read_inbound,
    write_json_frame,
};
use axagent_plugin_testkit::{command_frame, shutdown_frame};

const PLUGIN_ID: &str = "${plugin}@bundled";

struct Worker {
    child: Child,
    stdin: Option<BufWriter<ChildStdin>>,
    stdout: BufReader<ChildStdout>,
}

impl Worker {
    fn spawn() -> Self {
        Self::spawn_as(PLUGIN_ID)
    }

    fn spawn_as(injected_id: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_${binName}"))
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

    fn recv_request(&mut self) -> FrameRequest {
        match read_inbound(&mut self.stdout).expect("读帧应成功").as_request().cloned() {
            Some(request) => request,
            None => panic!("期望一帧请求（worker → 宿主），实际是响应"),
        }
    }

    fn recv_response(&mut self) -> FrameResponse {
        match read_inbound(&mut self.stdout).expect("读帧应成功").as_response().cloned() {
            Some(response) => response,
            None => panic!("期望一帧响应，实际是请求"),
        }
    }

    fn close_stdin(&mut self) {
        drop(self.stdin.take());
    }

    fn expect_clean_exit(&mut self) {
        self.send(&shutdown_frame());
        assert_eq!(self.recv_response().kind, ResponseKind::Success);
        self.close_stdin();
        assert!(self.child.wait().expect("等待 worker 退出").success());
    }
}

fn command(name: &str, input: serde_json::Value) -> FrameRequest {
    CommandFrame::new(PLUGIN_ID, name, input, 5_000).to_frame_request().expect("命令帧应能编成请求")
}

#[test]
fn describe_declares_both_outbound_seams() {
    let mut worker = Worker::spawn();
    worker.send(&FrameRequest::new(ops::DESCRIBE, serde_json::Value::Null).with_id("req-desc"));
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Success, "describe 应成功：{response:?}");
    let declaration = response.value.expect("describe 必须带声明");
    let calls = declaration["calls"].as_array().expect("calls 应是数组");
    for seam in ["opc.store", "tool.set"] {
        assert!(calls.iter().any(|value| value.as_str() == Some(seam)), "声明缺少出站接缝 {seam}");
    }
    worker.expect_clean_exit();
}

#[test]
fn tool_commands_forward_to_tool_set_exec_with_verbatim_args() {
    let cases = [
${cases}
    ];
    for (index, (command_name, tool_name, input)) in cases.into_iter().enumerate() {
        let mut worker = Worker::spawn();
        worker.send(&command(command_name, input.clone()).with_id(format!("req-{index}")));

        let call = worker.recv_request();
        assert_eq!(call.op, ops::CALL_SEAM);
        assert_eq!(call.args["seam"], "tool.set");
        assert_eq!(call.args["op"], "exec");
        assert_eq!(call.args["args"]["name"], tool_name, "工具名必须逐字转发");
        assert_eq!(call.args["args"]["args"], input, "工具入参必须原样透传");

        worker.send_response(
            &FrameResponse::success(serde_json::json!({ "ok": true, "tool": tool_name }))
                .with_id(call.request_id.clone().expect("出站请求必须带 request_id")),
        );
        let response = worker.recv_response();
        assert_eq!(response.kind, ResponseKind::Success, "{command_name} 应成功：{response:?}");
        let reply = response.value.expect("命令回执必须带 value");
        assert_eq!(reply["plugin_id"], PLUGIN_ID);
        assert_eq!(reply["value"]["tool"], tool_name);
        worker.expect_clean_exit();
    }
}

#[test]
fn automation_rules_inject_fixed_domain_id() {
    let mut worker = Worker::spawn();
    // 调用方故意塞 \`capabilityPackId\` 想越域；worker 必须用自身域 id 覆盖。
    worker.send(
        &command(
            "${runCommand}",
            serde_json::json!({
                "capabilityPackId": "security",
                "entityType": "${domain}",
                "entityId": "demo-1",
                "overdueDays": 3,
                "fields": { "stage": "示例" },
            }),
        )
        .with_id("req-rules"),
    );

    let call = worker.recv_request();
    assert_eq!(call.op, ops::CALL_SEAM);
    assert_eq!(call.args["seam"], "opc.store");
    assert_eq!(call.args["op"], "run_automation_rules");
    assert_eq!(call.args["args"]["capability_pack_id"], "${domain}", "不得让调用方越域");
    assert_eq!(call.args["args"]["entity_type"], "${domain}");
    assert_eq!(call.args["args"]["entity_id"], "demo-1");
    assert_eq!(call.args["args"]["overdue_days"], 3, "camelCase 必须转成接缝 snake_case");
    assert_eq!(call.args["args"]["fields"]["stage"], "示例");
    assert!(call.args["args"].get("capabilityPackId").is_none(), "接缝不得混入命令面键名");

    worker.send_response(
        &FrameResponse::success(serde_json::json!({ "matchedRules": 2 }))
            .with_id(call.request_id.clone().expect("出站请求必须带 request_id")),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Success);
    assert_eq!(response.value.expect("回执须带 value")["value"]["matchedRules"], 2);
    worker.expect_clean_exit();
}

${batchTest}#[test]
fn malformed_inputs_and_wrong_identity_are_rejected() {
    let mut worker = Worker::spawn();
    worker
        .send(&command("${first.command}", serde_json::json!([1, 2])).with_id("req-shape"));
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Error, "非对象入参必须在命令层被拒");
    assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_COMMAND_INVALID_ARGS));
    worker.expect_clean_exit();

    let mut worker = Worker::spawn();
    worker.send(&command("${plugin}_no_such_command", serde_json::json!({})).with_id("req-unknown"));
    let response = worker.recv_response();
    assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_COMMAND_UNKNOWN));
    worker.expect_clean_exit();

    // 错误身份必须是任何域都不使用的 id：早期模板写死 opc-security，
    // security 自身翻牌后该用例就不再是「错的身份」而会假绿。
    let mut worker = Worker::spawn_as("unrelated-plugin@bundled");
    worker.send(&command("${first.command}", serde_json::json!({})).with_id("req-id"));
    let response = worker.recv_response();
    assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_COMMAND_ID_MISMATCH));
    worker.expect_clean_exit();
}

#[test]
fn seam_error_is_propagated_without_fabricating_success() {
    let mut worker = Worker::spawn();
    worker
        .send(&command("${first.command}", serde_json::json!({})).with_id("req-seam-error"));
    let call = worker.recv_request();
    worker.send_response(
        &FrameResponse::error("TEST_TOOL_SEAM_UNAVAILABLE", "宿主未注册工具集")
            .with_id(call.request_id.clone().expect("出站请求必须带 request_id")),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Error);
    assert_eq!(response.code.as_deref(), Some("TEST_TOOL_SEAM_UNAVAILABLE"));
    assert_eq!(response.request_id.as_deref(), Some("req-seam-error"));
    worker.expect_clean_exit();
}

// 显式使用 testkit 的 \`command_frame\` 别名，保证测试与宿主 spawn 使用的帧构造器同源。
#[test]
fn testkit_command_frame_is_compatible_with_worker() {
    let mut worker = Worker::spawn();
    let request = command_frame(PLUGIN_ID, "${first.command}", serde_json::json!({}), 5_000)
        .with_id("req-testkit");
    worker.send(&request);
    let call = worker.recv_request();
    assert_eq!(call.args["op"], "exec");
    worker.send_response(
        &FrameResponse::success(serde_json::Value::Null)
            .with_id(call.request_id.expect("出站请求必须带 request_id")),
    );
    assert_eq!(worker.recv_response().kind, ResponseKind::Success);
    worker.expect_clean_exit();
}
`;
}

function packageJson() {
  return `{
  "name": "plugin-${plugin}-web",
  "private": true,
  "version": "0.0.0",
  "description": "OPC ${domain} 插件 bundle 页面：esbuild 把 src-web/main.ts 打成 web/pages/bundle.js",
  "scripts": {
    "build:web": "node ../../../node_modules/esbuild/bin/esbuild src-web/main.ts --bundle --format=iife --target=es2022 --outfile=web/pages/bundle.js"
  },
  "devDependencies": {
    "esbuild": "^0.28.2"
  }
}
`;
}

const TSCONFIG = `{
  "compilerOptions": {
    "target": "ES2022",
    "lib": ["ES2022", "DOM"],
    "module": "ESNext",
    "moduleResolution": "bundler",
    "strict": true,
    "noEmit": true,
    "skipLibCheck": true,
    "isolatedModules": true
  },
  "include": ["src-web/**/*.ts", "vendor/sdk/**/*.ts"]
}
`;

function webMainTs() {
  const samples = toolCommands
    .map(({ command, tool }) => `  ${JSON.stringify(tool)}: { command: ${JSON.stringify(command)}, input: ${sample(tool, "ts")} },`)
    .join("\n");
  const options = toolCommands
    .map(({ tool }) => `        <option value="${tool}">${tool}</option>`)
    .join("\n");
  // 特殊域才出现：批量校验样例是实体数组，仍从同一个命令分发表执行。
  const storeSamples = storeCommands.length
    ? `  "BatchValidateEntities": { command: ${JSON.stringify(storeCommands[0])}, input: ${sample("BatchValidateEntities", "ts")} },
`
    : "";
  const storeOptions = storeCommands.length
    ? `        <option value="BatchValidateEntities">批量校验实体</option>\n`
    : "";
  return `// SPDX-License-Identifier: AGPL-3.0-only
//
// ${domain} 插件页面入口：静态域数据由 domain-data.js 渲染，这里补上 worker 的交互区。
// 所有命令都走插件 SDK 的 executeCommand，不直接 invoke 宿主 IPC。

import { createAxAgentPluginClient } from "../vendor/sdk/pluginClientBridge";

const client = createAxAgentPluginClient();
client.announceReady();

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function render(value: unknown): string {
  if (typeof value === "string") {
    try {
      return JSON.stringify(JSON.parse(value), null, 2);
    } catch {
      return value;
    }
  }
  return JSON.stringify(value, null, 2);
}

const SAMPLES: Record<string, { command: string; input: unknown }> = {
${samples}${storeSamples}
  // \`capabilityPackId\` / \`domain_pack_id\` 由 worker 按自身身份固定注入；页面只提供实体字段。
  rules: {
    command: ${JSON.stringify(runCommand)},
    input: { entityType: ${JSON.stringify(domain)}, entityId: "demo" },
  },
};

/** 交互区自建在 body 末尾，避免与生成器产出的静态区模板耦合。 */
function mount(): void {
  const section = document.createElement("section");
  section.id = "${domain}-worker-panel";
  section.innerHTML = \`
    <h2>域 worker 命令</h2>
    <p class="muted">命令由 ${plugin} 的 Rust worker 声明；工具逻辑复用宿主实现。</p>
    <label>命令
      <select id="worker-command">
${options}${storeOptions}
        <option value="rules">触发自动化规则</option>
      </select>
    </label>
    <label>输入（JSON 对象）
      <textarea id="worker-input" rows="4" spellcheck="false"></textarea>
    </label>
    <button type="button" id="worker-run">执行命令</button>
    <pre id="worker-output" class="worker-output">尚未执行命令</pre>
  \`;
  document.body.append(section);

  const select = section.querySelector<HTMLSelectElement>("#worker-command");
  const input = section.querySelector<HTMLTextAreaElement>("#worker-input");
  const output = section.querySelector<HTMLElement>("#worker-output");
  const run = section.querySelector<HTMLButtonElement>("#worker-run");
  if (!select || !input || !output || !run) { return; }

  const fillSample = (): void => {
    const sample = SAMPLES[select.value];
    if (sample) { input.value = JSON.stringify(sample.input, null, 2); }
    output.textContent = "尚未执行命令";
  };
  select.addEventListener("change", fillSample);
  fillSample();

  run.addEventListener("click", () => void (async () => {
    const sample = SAMPLES[select.value];
    if (!sample) { return; }
    let args: unknown;
    try {
      args = JSON.parse(input.value) as unknown;
    } catch (error) {
      output.textContent = \`输入不是合法 JSON：\${errorText(error)}\`;
      return;
    }
    if (args === null || typeof args !== "object" || Array.isArray(args)) {
      output.textContent = "输入必须是 JSON 对象";
      return;
    }
    output.textContent = \`执行 \${sample.command}…\`;
    try {
      const result = await client.executeCommand({ name: sample.command, input: args });
      output.textContent = \`\${sample.command}\n\${render(result)}\`;
    } catch (error) {
      output.textContent = \`\${sample.command} 失败：\${errorText(error)}\`;
    }
  })());
}

mount();
`;
}

function verifyBundleMjs() {
  const commands = entry.commands.map((command) => `  "${command}",`).join("\n");
  return `// SPDX-License-Identifier: AGPL-3.0-only
/** ${domain} 页面 bundle 的最小静态校验：页面命令只能走 executeCommand。 */
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const bundle = readFileSync(join(root, "web/pages/bundle.js"), "utf8");
const commands = [
${commands}
];
for (const command of commands) {
  if (!bundle.includes(\`"\${command}"\`)) { throw new Error(\`bundle 未包含命令 \${command}\`); }
}
if (/client\\.invoke\\(\\s*["']opc_/.test(bundle)) {
  throw new Error('${domain} 页面不得使用 client.invoke("opc_*")');
}
console.log("PASS: ${domain} bundle 页面命令均走 executeCommand");
`;
}

/**
 * 主仓模板产物必须与独立仓 `cargo fmt --all` 后的源码逐字节相同：
 * 独立仓 CI 有 `cargo fmt --all --check`，而模板 crate 被 plugins workspace exclude。
 * 若只在同步阶段格式化，生成器自己的 --check 就会与主仓产物永远漂移。
 */
function formatRust(content, rel) {
  const result = spawnSync("rustfmt", ["--edition", "2024"], {
    input: content,
    encoding: "utf8",
    windowsHide: true,
  });
  if (result.error || result.status !== 0) {
    fail(`${rel} rustfmt 失败：${result.stderr || result.error?.message || "未知原因"}`);
  }
  return result.stdout;
}

const files = new Map([
  ["Cargo.toml", cargoToml()],
  ["src/main.rs", mainRs()],
  ["tests/protocol.rs", protocolRs()],
  ["package.json", packageJson()],
  ["tsconfig.json", TSCONFIG],
  ["src-web/main.ts", webMainTs()],
  ["src-web/__verify__/verify-bundle.mjs", verifyBundleMjs()],
]);
for (const rel of ["src/main.rs", "tests/protocol.rs"]) {
  files.set(rel, formatRust(files.get(rel), rel));
}
if (!existsSync(templateDir)) fail(`缺少 vendor SDK 模板目录：${templateDir}`);
for (const name of readdirSync(templateDir)) {
  const source = join(templateDir, name);
  files.set(`vendor/sdk/${name}`, readFileSync(source, "utf8"));
}

const problems = [];
for (const [rel, content] of files) {
  const path = join(targetDir, rel);
  if (CHECK) {
    if (!existsSync(path)) problems.push(`${rel}：文件缺失`);
    else if (readFileSync(path, "utf8") !== content) problems.push(`${rel}：与生成器输出不一致`);
  } else {
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, content);
  }
}
if (CHECK) {
  if (problems.length) {
    fail(`${plugin} 源码与生成器不一致：\n  - ${problems.join("\n  - ")}`);
  }
  console.log(`OPC_WORKER_SOURCE_OK ${domain}：${files.size} 个生成文件逐字节一致`);
} else {
  console.log(`OPC_WORKER_SOURCE_WRITTEN ${domain}：${files.size} 个生成文件`);
}
