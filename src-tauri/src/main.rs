// SPDX-License-Identifier: AGPL-3.0-only

// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

fn main() {
    // P1-NEW-3: 全局 panic hook，记录崩溃调用栈到 crash 日志文件。
    // 顺序铁律：先落盘 → 再 stderr（default_hook）→ 最后 tracing。
    // 背景：2026-09-06 嵌套 panic 事故——hook 里 tracing::error! 排在最前，
    // 第一 panic 若与 tracing/TLS 状态冲突，hook 自身再 panic，导致
    // "thread panicked while processing panic. aborting."，第一现场全丢
    // （crash log 从未生成）。
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let payload = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "Unknown panic payload".to_string()
        };

        let location = panic_info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown location".to_string());

        let thread = std::thread::current().name().unwrap_or("<unnamed>").to_string();
        let backtrace = std::backtrace::Backtrace::force_capture();
        let msg =
            format!("\n===== PANIC [{thread}] at {location}: {payload}\nBacktrace:\n{backtrace}\n");

        // 1. 先写 crash 日志文件（追加模式，保留历史崩溃记录）。
        //    这一步不依赖 tracing/任何运行时状态，是最可靠的兜底。
        let crash_log_path = std::env::var("APPDATA")
            .or_else(|_| std::env::var("HOME"))
            .map(|dir| PathBuf::from(dir).join("axagent-crash.log"))
            .unwrap_or_else(|_| PathBuf::from("axagent-crash.log"));
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&crash_log_path)
            .and_then(|mut f| std::io::Write::write_all(&mut f, msg.as_bytes()));

        // 2. 调用默认 hook（打印到 stderr，不依赖 tracing subscriber）。
        default_hook(panic_info);

        // 3. 最后写 tracing——subscriber 状态异常时也不影响前两步。
        tracing::error!("{msg}");
    }));

    axagent_lib::run()
}
