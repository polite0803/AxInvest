// SPDX-License-Identifier: AGPL-3.0-only

//! 插件 IPC 形态基准 —— 决策用，非统计用基准。
//!
//! 目的：为「A 层进程内（cdylib 等价）/ B 层长驻子进程 / per-call spawn」
//! 三选一提供**实测数**，替代量级估算。默认 1000 次往返。
//!
//! ## 运行
//!
//! ```text
//! cargo bench --bench plugin_ipc_bench
//! cargo bench --bench plugin_ipc_bench -- --reps 2000 --payload 1048576
//! ```
//!
//! 也可脱离整包编译直接跑（三平台横向对比时推荐，无需编译整个 axagent）：
//!
//! ```text
//! rustc --edition 2021 -O src-tauri/benches/plugin_ipc_bench.rs -o /tmp/pipc
//! /tmp/pipc --reps 1000 --payload 256
//! ```
//!
//! ## 为什么不引入 criterion
//!
//! 三种形态是**类别对比**而非参数化分布，且 per-call spawn 在 criterion 的多轮
//! 采样下会耗时数分钟。`harness = false` 正是为保留自打印输出的能力。
//!
//! ## 测得的分解关系
//!
//! ```text
//! inproc      = 插件计算下限（无传输）
//! pipe        = 长驻子进程（传输 + 计算）  ⇒ pipe   - inproc = 管道往返成本
//! spawn       = per-call（进程创建 + 传输 + 计算）⇒ spawn - pipe = 进程创建成本
//! spawn_bare  = 仅进程创建 + 退出（无负载）⇒ 隔离 CreateProcess/exec 的固定成本
//! ```
//!
//! 本基准不测 JSON 序列化本身的成本（需引 serde）。序列化量级约 1–10 µs，
//! 远低于传输成本，故不纳入——本文件的目的是量出**传输与进程创建的增量**。

use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

// ───────────────────────────── 长度前缀分帧协议 ─────────────────────────────

/// 写入一帧：4 字节大端长度 + 负载，随即 flush（否则对端会一直等）。
fn write_frame(w: &mut impl Write, payload: &[u8]) -> io::Result<()> {
    w.write_all(&(payload.len() as u32).to_be_bytes())?;
    w.write_all(payload)?;
    w.flush()
}

/// 读取一帧：先读 4 字节长度，再读足量负载。
fn read_frame(r: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

// ───────────────────────────── 被测量的「插件逻辑」 ─────────────────────────────

/// 模拟插件端的处理：O(负载) 的滚动哈希 + 回显。
///
/// 刻意做成 O(n) —— 这样大负载（如 `workflow.evolver` 的种群）能真实反映
/// 「序列化/传输成本盖过 IPC」的临界点。
fn handle(payload: &[u8]) -> Vec<u8> {
    let mut sum: u64 = 0;
    for &b in payload {
        sum = sum.wrapping_mul(31).wrapping_add(b as u64);
    }
    let mut out = Vec::with_capacity(payload.len() + 40);
    out.extend_from_slice(format!("{{\"sum\":{sum},\"echo\":").as_bytes());
    out.extend_from_slice(payload);
    out.push(b'}');
    out
}

/// 构造近似真实接缝请求的负载：固定头部 + 填充至 n 字节。
fn build_payload(n: usize) -> Vec<u8> {
    const HEAD: &[u8] = b"{\"seam\":\"workflow.business_rule\",\"op\":\"evaluate\",\"node_type\":\"filter\",\"data\":\"";
    let mut v = Vec::with_capacity(n + 16);
    v.extend_from_slice(HEAD);
    while v.len() + 4 < n {
        v.push(b'x');
    }
    v.extend_from_slice(b"\"}");
    v
}

// ───────────────────────────── 子进程模式 ─────────────────────────────

/// 子进程主体：`once == false` 时长驻循环，`once == true` 时处理一帧即退出
/// （后者用于复现 per-call spawn，对齐现状 `hooks.rs` 的调用形态）。
fn run_worker(once: bool) -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut r = stdin.lock();
    let mut w = stdout.lock();

    if !once {
        report_own_rss();
    }

    // 读失败（父进程关闭管道）即退出，这是正常的终止路径。
    while let Ok(req) = read_frame(&mut r) {
        let res = handle(&req);
        write_frame(&mut w, &res)?;
        if once {
            break;
        }
    }
    Ok(())
}

/// 在子进程内部报告自身的常驻内存，用于回答「每插件内存增量」。
///
/// 仅 Linux 实现（读 `/proc/self/statm`，免依赖）。Windows / macOS 请用
/// 任务管理器 / 活动监视器观察，或后续接 `GetProcessMemoryInfo` / `task_info`。
fn report_own_rss() {
    #[cfg(target_os = "linux")]
    {
        if let Ok(s) = std::fs::read_to_string("/proc/self/statm")
            && let Some(pages) = s.split_whitespace().nth(1).and_then(|v| v.parse::<u64>().ok())
        {
            // x86_64/aarch64 Linux 页大小通常为 4096。
            eprintln!("[worker] 常驻内存 ≈ {} KB", pages * 4);
        }
    }
}

fn spawn_worker(exe: &Path, once: bool) -> io::Result<Child> {
    let mut cmd = Command::new(exe);
    cmd.arg(if once { "--worker-once" } else { "--worker" });
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());

    // CREATE_NO_WINDOW：避免每次 spawn 弹出控制台窗口（对齐 hooks.rs 的做法）。
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    cmd.spawn()
}

// ───────────────────────────── 三种形态的测量 ─────────────────────────────

/// 形态 1：进程内直调（A 层 / cdylib 的**计算下限**，不含任何传输）。
fn bench_inproc(req: &[u8], reps: usize) -> Duration {
    let start = Instant::now();
    let mut acc = 0usize;
    for _ in 0..reps {
        acc = acc.wrapping_add(handle(req).len());
    }
    let elapsed = start.elapsed();
    std::hint::black_box(acc);
    elapsed
}

/// 形态 2：长驻子进程，复用同一对管道做 `reps` 次往返。
fn bench_pipe(exe: &Path, req: &[u8], reps: usize) -> io::Result<Duration> {
    let mut child = spawn_worker(exe, false)?;
    let mut tx: ChildStdin = child.stdin.take().expect("stdin 已 piped");
    let mut rx: ChildStdout = child.stdout.take().expect("stdout 已 piped");

    // 预热一次，排除首次管道建立/子进程初始化的固定成本。
    write_frame(&mut tx, req)?;
    let _ = read_frame(&mut rx)?;

    let start = Instant::now();
    for _ in 0..reps {
        write_frame(&mut tx, req)?;
        let _ = read_frame(&mut rx)?;
    }
    let elapsed = start.elapsed();

    // 关闭 stdin 会让子进程读到 EOF 后自行退出。
    drop(tx);
    let _ = child.wait();
    Ok(elapsed)
}

/// 形态 3：per-call spawn —— 每次调用新建一个进程（现状 `hooks.rs` 的形态）。
fn bench_spawn_per_call(exe: &Path, req: &[u8], reps: usize) -> io::Result<Duration> {
    let start = Instant::now();
    for _ in 0..reps {
        let mut child = spawn_worker(exe, true)?;
        {
            let mut tx = child.stdin.take().expect("stdin 已 piped");
            write_frame(&mut tx, req)?;
            // tx 在此 drop ⇒ 子进程不会等到 EOF 才结束
        }
        if let Some(mut rx) = child.stdout.take() {
            let _ = read_frame(&mut rx)?;
        }
        let _ = child.wait();
    }
    Ok(start.elapsed())
}

/// 形态 4：仅进程创建 + 退出，无负载 —— 隔离 CreateProcess / exec 的固定成本。
fn bench_spawn_bare(exe: &Path, reps: usize) -> io::Result<Duration> {
    let start = Instant::now();
    for _ in 0..reps {
        let mut child = spawn_worker(exe, true)?;
        // 立刻关闭 stdin：子进程读到 EOF 即退出，不处理任何负载。
        drop(child.stdin.take());
        let _ = child.wait();
    }
    Ok(start.elapsed())
}

// ───────────────────────────── 输出 ─────────────────────────────

fn report(label: &str, d: Duration, reps: usize) {
    let per = d.as_secs_f64() * 1e6 / reps as f64;
    println!("  {:<34} {:>9.2} ms 总计   {:>10.1} µs/次", label, d.as_secs_f64() * 1e3, per);
}

fn arg_usize(args: &[String], key: &str) -> Option<usize> {
    let i = args.iter().position(|a| a == key)?;
    args.get(i + 1)?.parse().ok()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--worker") {
        if let Err(e) = run_worker(false) {
            eprintln!("worker 失败: {e}");
            std::process::exit(1);
        }
        return;
    }
    if args.iter().any(|a| a == "--worker-once") {
        if let Err(e) = run_worker(true) {
            eprintln!("worker 失败: {e}");
            std::process::exit(1);
        }
        return;
    }

    let reps = arg_usize(&args, "--reps").unwrap_or(1000);
    let payload = arg_usize(&args, "--payload").unwrap_or(256);
    let exe = std::env::current_exe().expect("无法定位自身可执行文件");
    let req = build_payload(payload);

    println!("\n插件 IPC 形态基准");
    println!("  可执行文件 : {}", exe.display());
    println!("  往返次数   : {reps}");
    println!("  负载大小   : {payload} 字节\n");

    // 形态 1：进程内
    let inproc = bench_inproc(&req, reps);
    report("inproc（计算下限，无传输）", inproc, reps);

    // 形态 2：长驻子进程
    match bench_pipe(&exe, &req, reps) {
        Ok(d) => {
            report("pipe（长驻子进程）", d, reps);
            let delta = d.as_secs_f64() - inproc.as_secs_f64();
            println!(
                "  {:<34} {:>9.2} ms 总计   {:>10.1} µs/次",
                "  └─ 净传输成本（pipe − inproc）",
                delta * 1e3,
                delta * 1e6 / reps as f64
            );
        },
        Err(e) => println!("  pipe 测量失败: {e}"),
    }

    // 形态 3：per-call spawn
    match bench_spawn_per_call(&exe, &req, reps) {
        Ok(d) => report("spawn（per-call，现状形态）", d, reps),
        Err(e) => println!("  spawn 测量失败: {e}"),
    }

    // 形态 4：仅进程创建
    match bench_spawn_bare(&exe, reps) {
        Ok(d) => report("spawn_bare（仅进程创建+退出）", d, reps),
        Err(e) => println!("  spawn_bare 测量失败: {e}"),
    }

    println!("\n判读方式：");
    println!("  · `pipe − inproc` 即长驻子进程相对进程内的**真实传输开销**；");
    println!("    与 LLM 推理（0.5–30 s）/ 网络（10–1000 ms）对比即可判断是否可感知。");
    println!("  · `spawn − pipe` 即 per-call spawn 多付的**进程创建成本**。");
    println!("    若该值占比很高，说明症结在 per-call spawn，而非子进程模式本身。");
    println!("  · 用 `--payload 1048576` 复测，可观察大负载下序列化盖过 IPC 的临界点。\n");
}
