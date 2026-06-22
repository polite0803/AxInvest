// SPDX-License-Identifier: AGPL-3.0-only

fn main() {
    // Enable cfg(mobile) when building for Android or iOS
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("android")
        || target.contains("ios")
        || cfg!(target_os = "android")
        || cfg!(target_os = "ios")
    {
        println!("cargo:rustc-cfg=mobile");
        println!("cargo:warning=Building for mobile target: {}", target);
    }

    // ── 主线程栈：Windows 默认 1MB，异步工作流 DAG 深度嵌套会导致栈溢出 ──
    // 提高到 8MB，与 tokio multi-thread worker 默认栈一致。
    // 相关崩溃日志: thread 'main' has overflowed its stack (STATUS_STACK_OVERFLOW)
    if cfg!(target_os = "windows") {
        println!("cargo:rustc-link-arg=/STACK:8388608");
    }

    println!("cargo::rustc-check-cfg=cfg(mobile)");
    tauri_build::build()
}
