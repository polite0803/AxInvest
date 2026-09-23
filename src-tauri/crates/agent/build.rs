// SPDX-License-Identifier: AGPL-3.0-only
// agent crate 的测试二进制通过 dev-dependencies 链接 tauri（features = ["test"]），
// 需要嵌入 Common Controls v6 manifest，否则 Windows 上 cargo test 启动即报
// STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139)。
// 参考: https://github.com/tauri-apps/tauri/issues/11028
//       https://github.com/tauri-apps/tauri/discussions/11179

fn main() {
    #[cfg(target_os = "windows")]
    {
        let is_tauri_workspace = std::env::var("__TAURI_WORKSPACE__").is_ok_and(|v| v == "true");
        let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
        let manifest =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("common-controls.manifest");

        // ⚠ 这两条声明**必须在 if 之外**（与主 crate `src-tauri/build.rs:32-36` 同型）。
        //
        // 若写进下面的 if 分支，则「`__TAURI_WORKSPACE__` 未设」的那次构建**不声明任何
        // 依赖** ⇒ cargo 对该 build script 只按「包内文件是否变化」判新鲜 ⇒ 之后即便带上
        // `__TAURI_WORKSPACE__=true`，本脚本也不会重跑、测试 exe 也不会重链 ⇒ 产物永久停在
        // **无 manifest** 的那一份上，执行即 `STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139)`。
        //
        // 实测（2026-09-19，本案）：
        //   `__TAURI_WORKSPACE__=true cargo test --workspace --no-run` → `Finished in 3.41s`
        //   （**零重编**），仍指向 `axagent_agent-d7faea145338409d.exe`，该 exe 无
        //   `RT_MANIFEST` ⇒ 随即执行必崩。同期 24 个 build script 单元中，无变量那次留下
        //   空 output 的单元此后从未被重跑（env 依赖无从记录）。
        //   ⚠ 只做 `cargo clean` 或换 `CARGO_TARGET_DIR` 能绕过，但那不是修复。
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rerun-if-env-changed=__TAURI_WORKSPACE__");

        if is_tauri_workspace && target_env == "msvc" {
            // lib unit tests（#[cfg(test)] 在 lib 内）链接时只接受 rustc-link-arg，
            // rustc-link-arg-tests 仅作用于 tests/ 集成测试，对 lib unit tests 无效。
            // 该参数会传播到依赖 agent 的 crate 的 bin 链接产物（cargo 依赖传播）。
            println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
            println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
        }
    }
}
