// SPDX-License-Identifier: AGPL-3.0-only

//! 任务 #59（修 M1 回归）端到端验证：**旧形态 worker** 的命名命令必须回退到
//! spawn-on-call 通道并**真的执行**，同时**新形态 worker** 的命令失败**不得**回退。
//!
//! 与 `e2e_command_frame_test.rs`（#49 正向用例）互补，本文件钉死三条口径：
//! ① 旧 worker（未实现 `ops::COMMAND`，对命令帧回 `PLUGIN_UNKNOWN_OP`）+ 声明 `commands[]`
//!    ⇒ 命令**仍能跑出结果**（走 spawn），且宿主留一条 warn；
//! ② 回退**只发生一次** —— worker 侧 `op=command` 标记恰好一行、脚本侧
//!    `command=greet` 标记恰好一行、回退 warn 恰好一条（三者互证，排除「帧 ⇄ spawn 双跑」）；
//! ③ 命令**已进过 worker 逻辑**后的失败（`PLUGIN_COMMAND_FAILED`）不回退 ⇒
//!    已执行的命令不会被二次派发（声明了真可执行命令串却**没有**落 marker，即为证据）。
//!
//! ⚠ 本文件的命令声明刻意用**真的可执行命令串**（`./commands/greet.<cmd|sh>`），
//! 与 `e2e_command_frame_test.rs` 里的 `worker://<名>` 占位串相反：那边用占位串
//! 防「静默跑错通道」，这里必须证明「回退后真的执行了」，占位串无法作证。
//!
//! 门控与既有 e2e 用例同口径：`AXAGENT_TEST_PLUGIN_SUBPROCESS=1` 才真起子进程；
//! 未设置时打印 SKIP 并直接返回，不伪造通过。

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axagent_plugins::{PluginError, PluginManager, PluginManagerConfig};
use serde_json::json;
use tracing::field::{Field, Visit};
use tracing::{Event, Metadata, Subscriber};

// ─────────────────────────── warn 捕获夹具 ───────────────────────────

/// 一条被捕获的 tracing 事件（是否 WARN + 目标模块 + 全部字段的文本化）。
#[derive(Debug, Clone)]
struct CapturedEvent {
    is_warn: bool,
    target: String,
    fields: String,
}

/// 极简 tracing 订阅器：把事件字段拼成文本收进内存，供测试断言「日志里有 warn」。
///
/// 只实现 `Subscriber` 的必要方法，不建 span 注册表（被测代码不在 span 内发日志）。
/// 用 `tracing::subscriber::with_default` 装到**当前线程**，不碰全局默认订阅器，
/// 因此与同二进制里并行跑的其它用例互不影响。
#[derive(Debug)]
struct CapturingSubscriber {
    events: Arc<parking_lot::Mutex<Vec<CapturedEvent>>>,
    next_span: AtomicU64,
}

impl CapturingSubscriber {
    fn with_capture(run: impl FnOnce()) -> Vec<CapturedEvent> {
        let events = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let subscriber = Self { events: events.clone(), next_span: AtomicU64::new(0) };
        tracing::subscriber::with_default(subscriber, run);
        events.lock().clone()
    }
}

/// 把事件字段逐个拼成 `name={:?} value` 文本（`message` 字段也在其中）。
#[derive(Debug)]
struct TextVisitor {
    parts: Vec<String>,
}

impl Visit for TextVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.parts.push(format!("{}={:?}", field.name(), value));
    }
}

impl Subscriber for CapturingSubscriber {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        let id = self.next_span.fetch_add(1, Ordering::Relaxed) + 1;
        tracing::span::Id::from_u64(id)
    }

    fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut visitor = TextVisitor { parts: Vec::new() };
        event.record(&mut visitor);
        let metadata = event.metadata();
        self.events.lock().push(CapturedEvent {
            is_warn: *metadata.level() == tracing::Level::WARN,
            target: metadata.target().to_string(),
            fields: visitor.parts.join(" "),
        });
    }

    fn enter(&self, _span: &tracing::span::Id) {}

    fn exit(&self, _span: &tracing::span::Id) {}
}

// ─────────────────────────── 插件夹具 ───────────────────────────

fn require_plugin_subprocess() -> bool {
    if std::env::var("AXAGENT_TEST_PLUGIN_SUBPROCESS").as_deref() == Ok("1") {
        return true;
    }
    eprintln!(
        "SKIP: 未设置 AXAGENT_TEST_PLUGIN_SUBPROCESS=1，跳过端到端旧 worker 回退测试。\
         \n      本测试**未通过，也未被验证** —— 跳过仅表示环境不允许跑子进程。"
    );
    false
}

fn temp_dir(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("plugins-legacyfb-{label}-{nanos}"))
}

/// 命令脚本的文件名（随平台变），与 `manager.rs` 的同类夹具同形态。
fn command_script_name() -> &'static str {
    if cfg!(windows) {
        "greet.cmd"
    } else {
        "greet.sh"
    }
}

/// 命令脚本内容：**追加**落标记（`>>`），于是标记行数 = 该脚本被 spawn 的次数。
///
/// 这是「回退只发生一次」的直接证据来源之一，故不能像 #47/#49 夹具那样用覆盖写。
fn command_script_content() -> &'static str {
    if cfg!(windows) {
        "@echo off\r\necho command=%CLAWD_TOOL_NAME%>> ran.marker\r\
         \necho command=%CLAWD_TOOL_NAME%\r\n"
    } else {
        "#!/bin/sh\ncat >/dev/null\nprintf 'command=%s\\n' \"$CLAWD_TOOL_NAME\" >> ran.marker\n\
         printf 'command=%s\\n' \"$CLAWD_TOOL_NAME\"\n"
    }
}

/// 「已装 worker + 声明一条真可执行命名命令」的插件夹具。
///
/// `worker_bin_env` 选夹具二进制：
/// - `CARGO_BIN_EXE_e2e_legacy_worker` ⇒ 旧形态（未实现命令帧，回 `PLUGIN_UNKNOWN_OP`）；
/// - `CARGO_BIN_EXE_e2e_command_frame_worker` ⇒ 新形态（承接命令帧）；用于反证
///   「命令级失败不回退」。
struct FallbackFixture {
    manager: PluginManager,
    config_home: PathBuf,
    source: PathBuf,
    install_path: PathBuf,
    plugin_id: String,
}

impl FallbackFixture {
    fn new(label: &str, worker_exe: &Path) -> Self {
        let exe_name = worker_exe.file_name().expect("夹具 exe 应有文件名").to_string_lossy();
        let source = temp_dir(&format!("{label}-source"));
        let config_home = temp_dir(&format!("{label}-home"));
        fs::create_dir_all(source.join("worker")).expect("创建 worker 目录");
        fs::create_dir_all(source.join("commands")).expect("创建 commands 目录");
        fs::create_dir_all(source.join(".claude-plugin")).expect("创建 manifest 目录");
        fs::copy(worker_exe, source.join("worker").join(exe_name.as_ref()))
            .expect("夹具 exe 应拷贝进插件源目录");
        let script_path = source.join("commands").join(command_script_name());
        fs::write(&script_path, command_script_content()).expect("写入命令脚本");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&script_path).expect("脚本应有 metadata").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script_path, perms).expect("脚本应可 chmod");
        }

        // `command` 声明成 `./commands/<脚本>`：安装目录内的相对形态，由
        // `resolve_hook_entry` 解析后**真的能跑**（任务 #47 的语义）。
        // 这是本文件与 `e2e_command_frame_test.rs` 的关键差别 —— 那里用 `worker://` 占位串，
        // 一旦走错通道立刻失败；这里必须让回退后的 spawn 通道真跑出结果。
        let manifest = json!({
            "name": format!("{label}-demo"),
            "version": "1.0.0",
            "description": "旧 worker 命令回退端到端夹具",
            "defaultEnabled": true,
            "permissions": ["subprocess_execution", "read"],
            "worker": { "program": format!("./worker/{exe_name}") },
            "commands": [
                {
                    "name": "greet",
                    "description": "回退通道示证（真可执行命令串）",
                    "command": format!("./commands/{}", command_script_name())
                }
            ]
        });
        fs::write(source.join(".claude-plugin").join("plugin.json"), manifest.to_string())
            .expect("写 manifest");

        let mut manager = PluginManager::new(PluginManagerConfig::new(&config_home));
        let outcome = manager
            .install(source.to_str().expect("utf8 path"))
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

    /// 夹具 worker 记下的每个入站 op（一行一个）。
    fn worker_op_marker(&self) -> PathBuf {
        self.install_path.join("legacy_op.marker")
    }

    /// spawn 脚本记下的每次真实执行（一行一次）。
    fn spawn_marker(&self) -> PathBuf {
        self.install_path.join("ran.marker")
    }

    /// #49 新形态夹具 worker 处理命令帧前落的标记（`e2e_command_frame_worker.rs` 的 `note_ran`）。
    fn command_frame_marker(&self) -> PathBuf {
        self.install_path.join("command_frame.marker")
    }

    /// 标记文件里的指定行数（文件不存在 = 0 行，正是「没跑过」的证据形态）。
    fn marker_lines(&self, path: &Path, expected: &str) -> usize {
        match fs::read_to_string(path) {
            Ok(text) => text.lines().filter(|line| *line == expected).count(),
            Err(_) => 0,
        }
    }

    fn cleanup(self) {
        // 先放手 manager：随栈析构 kill worker，再删目录（Windows 上占用中的 exe 删不掉）。
        drop(self.manager);
        let _ = fs::remove_dir_all(&self.config_home);
        let _ = fs::remove_dir_all(&self.source);
    }
}

// ─────────────────────────── 用例 ───────────────────────────

/// 口径 ①＋②：旧 worker 的命名命令回退到 spawn 通道**真的执行**，且只执行一次。
#[test]
fn e2e_legacy_worker_falls_back_to_spawn_once() {
    if !require_plugin_subprocess() {
        return;
    }
    let worker_exe = PathBuf::from(env!("CARGO_BIN_EXE_e2e_legacy_worker"));
    let fixture = FallbackFixture::new("legacy-fb", &worker_exe);

    // 回退必须留 warn ⇒ 把执行整段包在捕获订阅器里（同线程、非全局）。
    let events = CapturingSubscriber::with_capture(|| {
        let output = fixture
            .manager
            .execute_plugin_command(&fixture.plugin_id, "greet", &json!({ "who": "world" }))
            .expect("旧 worker 不认识命令帧时命令应回退 spawn 通道并成功");
        assert!(output.contains("command=greet"), "spawn 通道输出应含脚本自报命令名：{output}");
    });

    // 只数「回退那一条」warn：同线程上沙箱权限告警（`note_network_access` 由
    // `build_sandbox_from_permissions` 发出，spawn 通道必经）同样留 WARN，
    // 按文案过滤才是在断言回退次数，而不是在断言无关告警的数量。
    let warns: Vec<&CapturedEvent> = events
        .iter()
        .filter(|event| {
            event.is_warn
                && event.target.starts_with("axagent_plugins")
                && event.fields.contains("spawn-on-call")
        })
        .collect();
    assert_eq!(warns.len(), 1, "回退 warn 应恰好一条，实际捕获 {events:?}");
    let warn_text = &warns[0].fields;
    for expected in [
        // 触发回退的协议错误码（结构化字段，不依赖消息文本的渲染形态）。
        "PLUGIN_UNKNOWN_OP",
        // 回退事由（完整落在同一个字符串片段内，见 manager.rs 的 warn 文案）。
        "未实现命令帧 op",
    ] {
        assert!(warn_text.contains(expected), "warn 应包含 `{expected}`，实际：{warn_text}");
    }
    // 实测锁（任务 #73 S-1 收口，替换原先写死的 `{code}: {message}` 整串）：
    // 真旧 worker 子进程的错误信封经宿主成形后，**结构化 `code` 字段**必须真的带上
    // `PLUGIN_UNKNOWN_OP` —— 那才是回退判据的输入本体。原断言锁的是文本首段
    // （PLAN §23.1 第 4 点据此写，被对抗复核判为措辞不实，见 §26.2 第 3 条）；
    // 文本形态如今不再参与判定，继续锁它就成了「双判据」，故改为锁字段。
    // 漂移方向因此从「改文案 ⇒ 回退静默消失」变成「构造点没填码 ⇒ 本断言先红」。
    assert!(
        warn_text.contains("error_code=PLUGIN_UNKNOWN_OP"),
        "旧 worker 的错误信封必须把码填进结构化字段（回退判据的实际输入），实际：{warn_text}"
    );
    // 原文本仍随 `worker 错误原文` 落进日志（可读性不降级），但不再被任何判据读取。
    assert!(
        warn_text.contains("未知操作：command"),
        "错误原文要能在日志里定位到 worker 自报的说明文本，实际：{warn_text}"
    );

    // 帧**确实派发过**（不是宿主偷偷跳过通道一），且只派发一次。
    assert_eq!(
        fixture.marker_lines(&fixture.worker_op_marker(), "op=command"),
        1,
        "命令帧应派发恰好一次（旧 worker 收到的 op 标记）"
    );
    // 回退后的 spawn **真的跑了**，且只跑一次 —— 与上一条合起来排除帧⇄spawn 双跑循环。
    assert_eq!(
        fixture.marker_lines(&fixture.spawn_marker(), "command=greet"),
        1,
        "回退后脚本应执行恰好一次（覆盖写无法计数，故脚本用 >> 追加）"
    );

    fixture.cleanup();
}

/// 口径 ③：新形态 worker 承接了命令帧、在命令逻辑**之后**自报失败
/// （`PLUGIN_COMMAND_UNKNOWN`）⇒ **不回退**：已执行的命令不得二次派发。
///
/// 夹具刻意声明了真可执行命令串，因此「没回退」是可观测的：脚本的 `ran.marker`
/// 不该出现，而不是靠错误类型推断。同时 #49 夹具 worker 的 `command_frame.marker`
/// **应当**出现（它落标记在分发命令之前）⇒ 一对正反证据：worker 侧已进命令逻辑、
/// 宿主侧没有再 spawn 一次。
#[test]
fn e2e_new_worker_command_failure_does_not_fall_back() {
    if !require_plugin_subprocess() {
        return;
    }
    let worker_exe = PathBuf::from(env!("CARGO_BIN_EXE_e2e_command_frame_worker"));
    let fixture = FallbackFixture::new("newworker-no-fb", &worker_exe);

    // 本夹具的 manifest 声明命令 `greet`；#49 夹具 worker 的命令表里没有它，
    // 于是 worker 回「认识命令帧、但没有这条命令的实现」= 命令级错误。
    let events = CapturingSubscriber::with_capture(|| {
        let err = fixture
            .manager
            .execute_plugin_command(&fixture.plugin_id, "greet", &json!({}))
            .expect_err("worker 回命令级错误必须上抛，不得被回退掩盖");
        assert!(matches!(&err, PluginError::CommandFailed(_)), "期望 CommandFailed，实际 {err:?}");
        let text = err.to_string();
        assert!(
            text.contains("PLUGIN_COMMAND_UNKNOWN"),
            "错误应透传 worker 的命令级错误码，实际：{text}"
        );
    });
    assert!(
        !events.iter().any(|event| { event.is_warn && event.fields.contains("回退到") }),
        "非能力拒绝的错误不该产生回退 warn，实际 {events:?}"
    );
    assert_eq!(
        fixture.marker_lines(
            &fixture.command_frame_marker(),
            &format!("ran command=greet plugin_id={}", fixture.plugin_id)
        ),
        1,
        "worker 应真的进过命令分发（落标记在按命令名分发之前）"
    );
    assert_eq!(
        fixture.marker_lines(&fixture.spawn_marker(), "command=greet"),
        0,
        "回退未触发 ⇒ 安装目录不该出现脚本标记"
    );

    fixture.cleanup();
}

/// 任务 #73 修 M-1：worker **装载成功之后才死**（`loaded_workers` 表项还在、子进程已没）
/// ⇒ 命名命令必须退回 spawn-on-call 通道并**真的执行一次**，而不是凭空失败。
///
/// 钉的是契约里那句「worker 中途异常退出也退回原通道」—— 修复前盘上没有这条路：
/// 命令走通道一拿到 transport 错误、不命中 `PLUGIN_UNKNOWN_OP` ⇒ 第 2 臂直接上抛，
/// M1 的失败形态只是换了触发条件（审查稿 A1.5）。
///
/// 与上一条 `e2e_legacy_worker_falls_back_to_spawn_once` 的分工：那条钉「能力拒绝 ⇒ 回退」，
/// 本条钉「帧**未送达** ⇒ 回退」；两条的判据函数不同（`command_error_is_unknown_op` vs
/// `command_error_is_worker_gone`），刻意不合并 —— 合并就是 M-1/M-2 的共同根。
#[test]
fn e2e_dead_worker_command_falls_back_to_spawn_once() {
    if !require_plugin_subprocess() {
        return;
    }
    let worker_exe = PathBuf::from(env!("CARGO_BIN_EXE_e2e_exit_after_handshake_worker"));
    let fixture = FallbackFixture::new("deadworker-fb", &worker_exe);

    // `enable()` 已返回 ⇒ describe 回过、表项已建立；现在等宿主**自己**判它死了
    // （判据与 `execute_command` 的存活预检同源，不是测试另造一套）。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while fixture.manager.plugin_worker_is_running(&fixture.plugin_id)
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        !fixture.manager.plugin_worker_is_running(&fixture.plugin_id),
        "夹具回完 describe 即退出；5s 内宿主仍判它活着 ⇒ 存活判据失灵，本用例无法证 M-1"
    );

    let events = CapturingSubscriber::with_capture(|| {
        let output = fixture
            .manager
            .execute_plugin_command(&fixture.plugin_id, "greet", &json!({ "who": "world" }))
            .expect("worker 进程已死时命名命令应回退 spawn 通道并成功（M-1 契约）");
        assert!(output.contains("command=greet"), "spawn 通道输出应含脚本自报命令名：{output}");
    });

    let warns: Vec<&CapturedEvent> = events
        .iter()
        .filter(|event| {
            event.is_warn
                && event.target.starts_with("axagent_plugins")
                && event.fields.contains("spawn-on-call")
        })
        .collect();
    assert_eq!(warns.len(), 1, "回退 warn 应恰好一条，实际捕获 {events:?}");
    let warn_text = &warns[0].fields;
    // 回退理由必须点名「进程已不在 + 帧未送达」，不能含糊成「worker 出错」。
    for expected in ["进程已不在", "命令帧未送达"] {
        assert!(warn_text.contains(expected), "warn 应包含 `{expected}`，实际：{warn_text}");
    }
    // 进程不在 ⇒ 没有能力拒绝可报 ⇒ 结构化码位是空串（S-1 之后判据读的就是这个字段）。
    assert!(warn_text.contains("error_code="), "warn 必须带结构化码位字段，实际：{warn_text}");

    // 回退后的 spawn **真的跑了**，且只跑一次；死 worker 侧一个标记都没落 ⇒ 没有双跑。
    assert_eq!(
        fixture.marker_lines(&fixture.spawn_marker(), "command=greet"),
        1,
        "回退后脚本应执行恰好一次"
    );
    assert!(
        !fixture.worker_op_marker().exists(),
        "夹具进程已退出，不可能收到过命令帧 ⇒ 不该有旧 worker 的 op 标记"
    );

    fixture.cleanup();
}
