// SPDX-License-Identifier: AGPL-3.0-only

//! 任务 #53（D1 修复）端到端验证：接缝调用在**真实长驻 worker** 上的等待预算。
//!
//! `e2e_command_frame_test.rs`（#49）钉的是命令通道；本文件钉的是接缝公共入口 ——
//! 逐条验收口径：
//! ① worker 对 `hang` 帧装死 ⇒ `LoadedPlugin::invoker().invoke(...)` 在预算内
//!    返回超时错误，**不是**永久挂住（D1 的端到端判据）；夹具 op `hang` **未进**
//!    plugin-proto 的 `SEAM_OP_TIMEOUT_OVERRIDES` ⇒ 本用例同时钉住
//!    「未进表 op 回落连接级 `seam_wait_timeout`」这条选取规则；
//! ② 超时后连接被标脏：第二次接缝调用被**立即**拒绝，不再排队等一个新预算；
//! ③ 标脏后的 `unload` 走急停路径仍在看门狗内收口（卸载不被拖挂）。
//!
//! 门控与 `e2e_worker_test.rs` 同口径：`AXAGENT_TEST_PLUGIN_SUBPROCESS=1` 才真起子进程；
//! 未设置时打印 SKIP，不伪造通过。

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use axagent_plugins::{LoadedPlugin, PluginWorkerConfig};
use serde_json::json;

fn require_plugin_subprocess() -> bool {
    if std::env::var("AXAGENT_TEST_PLUGIN_SUBPROCESS").as_deref() == Ok("1") {
        return true;
    }
    eprintln!(
        "SKIP: 未设置 AXAGENT_TEST_PLUGIN_SUBPROCESS=1，跳过端到端接缝超时子进程测试。\
         \n      本测试**未通过，也未被验证** —— 跳过仅表示环境不允许跑子进程。"
    );
    false
}

/// 接缝等待预算（测试注入值）：远小于协议缺省（15 s），又给 Windows 进程调度留余量。
const SEAM_WAIT: Duration = Duration::from_millis(300);

/// 验收①+②+③：真 worker 装死 ⇒ 预算内超时、二次调用被可疑闸拒、卸载不挂死。
#[test]
fn e2e_seam_call_times_out_within_budget_on_real_worker() {
    if !require_plugin_subprocess() {
        return;
    }
    let program = PathBuf::from(env!("CARGO_BIN_EXE_e2e_seam_timeout_worker"));
    let loaded = LoadedPlugin::load(
        PluginWorkerConfig::new("e2e-seam-timeout@external", program)
            .with_seam_wait_timeout(SEAM_WAIT),
    )
    .expect("真实 worker 应完成 spawn + describe 握手");
    let invoker = loaded.invoker();

    // ① 装死必须被接缝预算拦下（错误文本来自 WorkerError::Timeout 的 Display）。
    let started = Instant::now();
    let err = invoker.invoke("hang", json!({})).expect_err("worker 装死必须被预算拦下（D1 判据）");
    let elapsed = started.elapsed();
    assert!(err.contains("超过宿主等待上限"), "错误应点明超时，实际：{err}");
    assert!(err.contains("300 ms"), "预算应随错误带回，实际：{err}");
    assert!(
        elapsed >= SEAM_WAIT && elapsed < Duration::from_secs(3),
        "收口应落在预算附近（夹具装死 2.5 s），实际 {elapsed:?}"
    );

    // ② 标脏后的第二次接缝调用：被「可疑」闸拒，不得再等一个预算。
    let started = Instant::now();
    let err2 = invoker.invoke("hang", json!({})).expect_err("标脏后新接缝调用必须被拒");
    let refused_in = started.elapsed();
    assert!(err2.contains("可疑"), "错误应点明连接可疑，实际：{err2}");
    assert!(refused_in < SEAM_WAIT, "闸拒不得再等一个预算，实际 {refused_in:?}");

    // ③ 卸载必须在合理时间内收口：标脏路径跳过「等 shutdown 回执」的礼貌帧，
    //    否则装死的 worker 会把 unload 一起拖挂（D1 的卸载侧变体）。
    //    放看门狗线程里跑：真挂死时本用例判**失败**，而不是让测试进程卡住。
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        loaded.unload();
        let _ = sender.send(());
    });
    receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("标脏后的 unload 必须在 10 s 内收口（急停 + 限时等退出 + 强杀兜底）");
}
