// SPDX-License-Identifier: AGPL-3.0-only

//! 任务 #49（T2 派发口形态升级）端到端验证：长驻 worker + 命令帧。
//!
//! 与 `e2e_worker_test.rs`（接缝握手）互补：本文件证明**命名命令通道**从
//! 「spawn-on-call」切到「复用长驻 worker 的命令帧」后全链路可用，逐条钉死验收口径：
//! ① 命令帧往返（宿主绑定 pluginId 原样到达 worker 并带回）；
//! ② 无状态命令 `echo_input` 经新通道跑通（夹具不含业务逻辑，真股票的端到端在
//!    `e2e_stock_tool_test.rs` / `e2e_stock_bundle_install_test.rs`）；
//! ③ 输入形状不合法在**派发之前**被拒（worker 的落盘标记为零证明）；
//! ④ 超时被宿主拦下（预算 + 收帧宽限内收口，不级联卡死）；
//! ⑤ worker 自报失败透传协议错误码（上层据此映射 `ErrorResponse`）。
//!
//! 门控与 `e2e_worker_test.rs` 同口径：`AXAGENT_TEST_PLUGIN_SUBPROCESS=1` 才真起子进程；
//! 未设置时打印 SKIP，不伪造通过。

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use axagent_plugins::{PluginError, PluginManager, PluginManagerConfig};
use serde_json::{Value, json};

fn require_plugin_subprocess() -> bool {
    if std::env::var("AXAGENT_TEST_PLUGIN_SUBPROCESS").as_deref() == Ok("1") {
        return true;
    }
    eprintln!(
        "SKIP: 未设置 AXAGENT_TEST_PLUGIN_SUBPROCESS=1，跳过端到端命令帧子进程测试。\
         \n      本测试**未通过，也未被验证** —— 跳过仅表示环境不允许跑子进程。"
    );
    false
}

fn temp_dir(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("plugins-cmdframe-{label}-{nanos}"))
}

/// 「已安装 + 已启用（worker 长驻中）」的插件夹具。
struct WorkerPluginFixture {
    manager: PluginManager,
    config_home: PathBuf,
    source: PathBuf,
    install_path: PathBuf,
    plugin_id: String,
}

impl WorkerPluginFixture {
    /// 装一个「声明 worker + 三条命名命令」的插件并启用它。
    ///
    /// manifest 要点：
    /// - `worker.program` 指向**拷贝进源目录**的夹具 exe（worker 路径只允许安装目录内相对路径）；
    /// - `echo_input` 声明 `inputSchema.required=["text"]`，用于证明「形状不合法在派发前被拒」；
    ///   `sleep_too_long` / `fail_self` 不声明 schema；
    /// - `commands[].command` 用 `worker://<名>` 占位串：worker 通道不读它；一旦**意外**
    ///   退回 spawn 通道，这个不存在的程序名会立刻失败暴露，而不是静默跑别的东西。
    fn new(label: &str) -> Self {
        let exe = PathBuf::from(env!("CARGO_BIN_EXE_e2e_command_frame_worker"));
        let exe_name = exe.file_name().expect("夹具 exe 应有文件名").to_string_lossy();
        let source = temp_dir(&format!("{label}-source"));
        let config_home = temp_dir(&format!("{label}-home"));
        fs::create_dir_all(source.join("worker")).expect("创建 worker 目录");
        fs::create_dir_all(source.join(".claude-plugin")).expect("创建 manifest 目录");
        fs::copy(&exe, source.join("worker").join(exe_name.as_ref()))
            .expect("夹具 exe 应拷贝进插件源目录");

        let manifest = json!({
            "name": format!("{label}-demo"),
            "version": "1.0.0",
            "description": "T2 命令帧端到端夹具",
            "defaultEnabled": true,
            "permissions": ["subprocess_execution", "read"],
            "worker": { "program": format!("./worker/{exe_name}") },
            "commands": [
                {
                    "name": "echo_input",
                    "description": "回显 input.text（无状态命令示证）",
                    "command": "worker://echo_input",
                    "inputSchema": { "required": ["text"] }
                },
                {
                    "name": "sleep_too_long",
                    "description": "睡过预算（超时拦截示证）",
                    "command": "worker://sleep_too_long"
                },
                {
                    "name": "fail_self",
                    "description": "worker 自报失败（错误码透传示证）",
                    "command": "worker://fail_self"
                }
            ]
        });
        fs::write(source.join(".claude-plugin").join("plugin.json"), manifest.to_string())
            .expect("写 manifest");

        let mut manager = PluginManager::new(PluginManagerConfig::new(&config_home));
        let outcome = manager
            .install_test_fixture(source.to_str().expect("utf8 path"))
            .unwrap_or_else(|e| panic!("install 应成功: {e}"));
        manager
            .enable(&outcome.plugin_id)
            .unwrap_or_else(|e| panic!("enable（含 worker spawn）应成功: {e}"));

        Self {
            manager,
            config_home,
            source,
            install_path: outcome.install_path,
            plugin_id: outcome.plugin_id,
        }
    }

    /// worker 处理过命令才会落下的标记（安装在插件目录，宿主不参与）。
    fn marker(&self) -> PathBuf {
        self.install_path.join("command_frame.marker")
    }

    fn cleanup(self) {
        // 先放手 manager：随栈析构 kill worker，再删目录（Windows 上占用中的 exe 删不掉）。
        drop(self.manager);
        let _ = fs::remove_dir_all(&self.config_home);
        let _ = fs::remove_dir_all(&self.source);
    }
}

/// 验收①+②：无状态命令 `echo_input` 经命令帧跑通，且宿主绑定的
/// pluginId 原样到达 worker（落盘标记）与宿主（回执绑定字段随传输层往返）。
#[test]
fn e2e_command_frame_roundtrip_executes_stateless_stock_command() {
    if !require_plugin_subprocess() {
        return;
    }
    let fixture = WorkerPluginFixture::new("frame-ok");
    let output = fixture
        .manager
        .execute_plugin_command(
            &fixture.plugin_id,
            "echo_input",
            &json!({ "text": "调研茅台短线" }),
        )
        .expect("命令帧全链路应成功");

    let parsed: Value = serde_json::from_str(&output).expect("命令帧通道输出应为 JSON 文本");
    assert_eq!(parsed["text"], "调研茅台短线");
    assert_eq!(parsed["length"], 6, "字符数按 chars 计（中文 6 字，不是字节数）");

    let marker = fs::read_to_string(fixture.marker()).expect("worker 必须真的跑过这条命令");
    assert!(
        marker.contains(&format!("plugin_id={}", fixture.plugin_id)),
        "帧上的 pluginId 必须等于宿主注册表权威键，实际标记：{marker}"
    );
    assert!(marker.contains("command=echo_input"), "命令名必须原样到达 worker");
    fixture.cleanup();
}

/// 验收③：`inputSchema.required` 缺字段 ⇒ `CommandFailed`，且**派发前**被拒 ——
/// worker 的落盘标记为零（不存在）作客观证据，不是靠错误类型反推。
#[test]
fn e2e_command_frame_invalid_input_schema_rejected_before_dispatch() {
    if !require_plugin_subprocess() {
        return;
    }
    let fixture = WorkerPluginFixture::new("frame-schema");
    let err = fixture
        .manager
        .execute_plugin_command(&fixture.plugin_id, "echo_input", &json!({}))
        .expect_err("缺必填 text 必须在派发前被拒");
    assert!(matches!(&err, PluginError::CommandFailed(_)), "期望 CommandFailed，实际 {err:?}");
    let text = err.to_string();
    assert!(text.contains("缺少必填字段 `text`"), "错误应点明缺失字段，实际：{text}");
    assert!(text.contains("未派发命令帧"), "语义应写明未派发，实际：{text}");
    assert!(!fixture.marker().exists(), "被拒的命令不得到达 worker（安装目录不应出现执行标记）");
    fixture.cleanup();
}

/// 验收④：worker 睡过预算 ⇒ 宿主在「预算 + 收帧宽限」处拿 `CommandTimeout`，
/// 不级联卡死调用线程（D1 修复的端到端证据）。
#[test]
fn e2e_command_frame_timeout_is_intercepted_by_host() {
    if !require_plugin_subprocess() {
        return;
    }
    let fixture = WorkerPluginFixture::new("frame-timeout");
    let started = Instant::now();
    let err = fixture
        .manager
        .execute_plugin_command_with_timeout(
            &fixture.plugin_id,
            "sleep_too_long",
            &json!({ "sleepMs": 2_500 }),
            Duration::from_millis(200),
        )
        .expect_err("worker 装死必须被超时拦下");
    let elapsed = started.elapsed();
    assert!(matches!(&err, PluginError::CommandTimeout(_)), "期望 CommandTimeout，实际 {err:?}");
    assert!(err.to_string().contains("超过宿主等待上限"), "错误应写明超时，实际：{err}");
    assert!(
        elapsed >= Duration::from_millis(1_100) && elapsed < Duration::from_secs(5),
        "收口应落在预算+宽限附近（1.2s 上下），实际 {elapsed:?}"
    );
    fixture.cleanup();
}

/// 验收⑤：worker 自报失败 ⇒ 宿主错误的 Display 透传协议码 `PLUGIN_COMMAND_FAILED`
/// ——上层命令层据此映射 `ErrorResponse`（映射本体与用例在主 crate `commands/plugin.rs`）。
#[test]
fn e2e_command_frame_worker_fault_carries_protocol_code() {
    if !require_plugin_subprocess() {
        return;
    }
    let fixture = WorkerPluginFixture::new("frame-fault");
    let err = fixture
        .manager
        .execute_plugin_command(&fixture.plugin_id, "fail_self", &json!({}))
        .expect_err("worker 故意失败必须报 Err");
    assert!(matches!(&err, PluginError::CommandFailed(_)), "期望 CommandFailed，实际 {err:?}");
    assert!(
        err.to_string().contains("PLUGIN_COMMAND_FAILED"),
        "协议错误码必须透传到宿主侧错误，实际：{err}"
    );
    fixture.cleanup();
}
