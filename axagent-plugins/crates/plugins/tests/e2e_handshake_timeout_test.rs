// SPDX-License-Identifier: AGPL-3.0-only

//! 任务 #63（补 D1 的握手边角）端到端验证：**worker 起来了但不回 `describe`** 时，
//! 装载必须在握手预算内失败，而不是永久挂住整条插件装载路径。
//!
//! 逐条验收口径（与派单的硬要求一一对应）：
//! ① 装死 worker 的 [`LoadedPlugin::load`] 在收口硬上界内返回 `Err`，且错误文本同时
//!    点明「握手失败」与「`describe` 超过宿主等待上限 300 ms」⇒ **可定位**，不退化成
//!    笼统的通道错误；
//! ② 失败原因确是「不回帧」：夹具在 cwd 落的 `handshake_op.marker` 里有 `op=describe`
//!    （排除「进程没起来 / 管道没接上」这两种同样报握手失败的情形）；
//! ③ 装载失败**不留脏状态**：紧随其后用另一夹具（正常回 describe 的
//!    `e2e_seam_timeout_worker`）装载 ⇒ 必须成功并干净卸载。证明 #63 在握手超时上置的
//!    「连接可疑」闸只属于那条已被丢弃的连接，不会外泄到下一次装载（重试拿的是新 worker）；
//! ④ 判据形态：真挂死时本用例必须**失败**而不是让整个测试二进制卡住 ⇒ `load` 放在
//!    看门狗线程里跑，主线程 `recv_timeout` 收口。
//!
//! ⚠ 门控与 `e2e_seam_timeout_test.rs` 同口径：`AXAGENT_TEST_PLUGIN_SUBPROCESS=1` 才真起
//! 子进程；未设置时打印 SKIP 后返回 —— **SKIP 不等于通过**。不带门控时恒跑的护栏是
//! `src/worker.rs` 的 `handshake_times_out_within_budget_and_poisons_the_connection`
//! （回环 socket 夹具，同样证明「有界」，只是不含真进程/真管道）。

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use axagent_plugins::{LoadedPlugin, PluginWorkerConfig};

/// 装载收口的硬上界：夹具装死 20 s，任何落在本界内的返回都证明「有界」。
/// 放宽到 10 s 是给 Windows 进程调度与强杀留余量，不是给「等握手回帧」留余量。
const LOAD_HARD_LIMIT: Duration = Duration::from_secs(10);

/// 握手预算（测试注入值）：远小于协议缺省（plugin-proto 的
/// `DEFAULT_HANDSHAKE_TIMEOUT_MS` = 10 s），又给进程调度留可测的余量。
/// **产品常量不因测试而改小。**
const HANDSHAKE_WAIT: Duration = Duration::from_millis(300);

/// 停机上限（测试注入值）：握手超时后走急停，本字段即「关管道 + 限时等退出」的上界。
const SHUTDOWN_WAIT: Duration = Duration::from_millis(300);

/// 与既有 e2e 用例同口径的子进程门控：未开启时明确声明「未通过、也未被验证」。
fn require_plugin_subprocess() -> bool {
    if std::env::var("AXAGENT_TEST_PLUGIN_SUBPROCESS").as_deref() == Ok("1") {
        return true;
    }
    eprintln!(
        "SKIP: 未设置 AXAGENT_TEST_PLUGIN_SUBPROCESS=1，跳过端到端握手超时的子进程测试。\
         \n      本测试**未通过，也未被验证** —— 跳过仅表示环境不允许跑子进程。\
         \n      不永挂的恒跑护栏见 worker.rs 的 handshake_times_out_within_budget_and_poisons_the_connection。"
    );
    false
}

/// 子进程夹具的收口确认：等标记文件落到 `want` 行再放行。
///
/// **必须等**：第一次装载失败时夹具 worker 仍活着（它的 20 s 睡眠还没睡完，宿主只是
/// 强杀了自己那份句柄），若立刻起第二个同 ID 进程，本用例的判据就会落在不确定的
/// 时序上。超时不放行 ⇒ 让用例以失败收尾，而不是无声跳过这条前置。
fn wait_for_marker(dir: &Path, name: &str, want: &str, limit: Duration) {
    let deadline = Instant::now() + limit;
    loop {
        if read_marker(dir, name).contains(want) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "等待夹具标记 `{name}` 含 `{want}` 超时（{limit:?}），实际内容：{c}",
            c = read_marker(dir, name)
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// 在**另一条线程**里跑 `LoadedPlugin::load`，只把「结果文本」送回主线程。
///
/// 为什么不让主线程直接调用：缺陷复发时 load 会永久挂住，而挂住的线程会连带整个
/// 测试二进制超时（读者看到的是「测试没跑完」而不是「用例失败」）。放线程里 +
/// `recv_timeout` ⇒ 挂死判**失败**。`LoadedPlugin` 不跨线程交付，在 worker 线程内
/// 就地转成文本（成功则顺手 unload，不留常驻子进程）。
fn load_in_watchdog_thread(config: PluginWorkerConfig) -> Result<String, ()> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let verdict = match LoadedPlugin::load(config) {
            Ok(loaded) => {
                loaded.unload();
                "OK".to_string()
            },
            Err(e) => format!("ERR:{e}"),
        };
        let _ = sender.send(verdict);
    });
    receiver.recv_timeout(LOAD_HARD_LIMIT).map_err(|_| ())
}

/// 唯一化的临时安装目录（夹具把 op 标记落在自己的 cwd，宿主按安装目录设 cwd）。
fn make_install_dir(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("测试：系统时间应晚于 UNIX EPOCH")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("plugins-handshake-{label}-{nanos}"));
    std::fs::create_dir_all(&dir).expect("测试：应能创建临时安装目录");
    dir
}

/// 读标记文件内容（不存在时返回空串，让断言以「缺 op=describe」的形式失败）。
fn read_marker(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap_or_default()
}

/// 验收①+②：真 worker 不回 describe ⇒ 装载在预算内失败，错误可定位到握手。
#[test]
fn e2e_load_fails_within_handshake_budget_on_silent_worker() {
    if !require_plugin_subprocess() {
        return;
    }
    let program = PathBuf::from(env!("CARGO_BIN_EXE_e2e_no_describe_worker"));
    let dir = make_install_dir("silent");
    let config = PluginWorkerConfig {
        shutdown_timeout: SHUTDOWN_WAIT,
        ..PluginWorkerConfig::new("e2e-handshake-silent@external", program)
            .with_handshake_timeout(HANDSHAKE_WAIT)
            .with_working_dir(dir.clone())
    };

    let started = Instant::now();
    let verdict = load_in_watchdog_thread(config)
        .expect("装载必须在收口硬上界内返回（#63 判据：僵死 worker 不得把插件装载路径永久挂住）");
    let elapsed = started.elapsed();

    let text = match verdict.strip_prefix("ERR:") {
        Some(text) => text,
        None => panic!("装载必须失败，实际：{verdict}"),
    };
    // ① 可定位：既点明失败发生在**握手**阶段，又带未回帧的 op 与预算毫秒数。
    assert!(text.contains("握手失败"), "错误应点明失败在握手阶段，实际：{text}");
    assert!(text.contains("describe"), "错误应点名未回帧的 op，实际：{text}");
    assert!(
        text.contains("超过宿主等待上限 300 ms"),
        "错误应带回握手预算（不得退化成笼统通道错误），实际：{text}"
    );
    // 收口落在「握手预算 + 停机上限」附近，而不是夹具的 20 s 装死。
    assert!(
        elapsed >= HANDSHAKE_WAIT && elapsed < LOAD_HARD_LIMIT,
        "收口应落在预算附近，实际 {elapsed:?}"
    );
    // ② 失败原因确是「不回帧」：worker 收到过 describe（否则该报 Spawn/管道类错误）。
    assert!(
        read_marker(&dir, "handshake_op.marker").contains("op=describe"),
        "夹具应确实收到过 describe 帧（证明是装死而非没起来）"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 验收③：握手超时置的「可疑」闸不外泄 —— 装载失败后紧接着装载另一个 worker
/// 必须成功（新进程 + 新连接 + 闸=false），证明重试路径拿得到新 worker。
#[test]
fn e2e_handshake_timeout_does_not_block_the_next_load() {
    if !require_plugin_subprocess() {
        return;
    }
    let silent_program = PathBuf::from(env!("CARGO_BIN_EXE_e2e_no_describe_worker"));
    let normal_program = PathBuf::from(env!("CARGO_BIN_EXE_e2e_seam_timeout_worker"));

    // 第一次：装死夹具 ⇒ 必须失败（同上个用例，这里只取「失败」这一事实）。
    let silent_dir = make_install_dir("silent-retry");
    let silent_verdict = load_in_watchdog_thread(PluginWorkerConfig {
        shutdown_timeout: SHUTDOWN_WAIT,
        ..PluginWorkerConfig::new("e2e-handshake-retry@external", silent_program)
            .with_handshake_timeout(HANDSHAKE_WAIT)
            .with_working_dir(silent_dir.clone())
    })
    .expect("装死夹具的装载必须收口");
    assert!(
        silent_verdict.starts_with("ERR:"),
        "前置条件：第一次装载应失败，实际：{silent_verdict}"
    );
    // 前置确认：失败真的发生在「describe 已送达」之后，而不是进程压根没起来。
    wait_for_marker(&silent_dir, "handshake_op.marker", "op=describe", Duration::from_secs(5));

    // 第二次：正常回 describe 的夹具 ⇒ 必须装载成功并可卸载（不被上一次的标脏污染）。
    // `LoadedPlugin::load` 成功即蕴含 describe 握手已通过，无需第二个夹具另落标记。
    let normal_dir = make_install_dir("normal-retry");
    let normal_verdict = load_in_watchdog_thread(PluginWorkerConfig {
        shutdown_timeout: SHUTDOWN_WAIT,
        ..PluginWorkerConfig::new("e2e-handshake-next@external", normal_program)
            .with_handshake_timeout(HANDSHAKE_WAIT)
            .with_working_dir(normal_dir.clone())
    })
    .expect("紧随其后的装载必须收口（不得被前一次失败拖挂）");
    assert_eq!(
        normal_verdict, "OK",
        "上一次的握手超时不得污染下一次装载（标脏闸随连接生灭，重试拿新 worker）"
    );

    let _ = std::fs::remove_dir_all(&silent_dir);
    let _ = std::fs::remove_dir_all(&normal_dir);
}
