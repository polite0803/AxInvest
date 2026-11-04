#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * `tauri build` 前置：把插件 worker 产物暂存到 `src-tauri/plugins/bundled/`，
 * 供 `bundle.resources` 打进安装包（宿主按 `resource_dir/plugins/bundled` 发现，见
 * `PluginManagerConfig::resolve_bundled_root`）。
 *
 * 为什么落在 `src-tauri/plugins`（而不是 `src-tauri/resources/plugins`）：Tauri 打包
 * resources 时**保留相对配置目录的路径** ⇒ 声明 `"plugins"` 会落到 `<resource_dir>/plugins/…`，
 * 与宿主的解析路径逐字对齐；多套一层 `resources/` 会让宿主找不到。
 *
 * 为什么需要 target 守卫：`beforeBuildCommand` 同时服务 `tauri android/ios build`，
 * 而 `cargo build -p axagent-plugin-stock` 编出的是**宿主架构**的可执行文件 —— 打进移动端
 * 包里毫无用处（还白增体积）。因此只在桌面平台暂存；移动端直接跳过并退 0。
 *
 * 识别依据是 Tauri CLI 注入的环境变量（`TAURI_ENV_PLATFORM` / `TAURI_ENV_TARGET_TRIPLE`）。
 * 两个都没有时（手工直跑本脚本）按「桌面」处理 —— 手工运行本来就是想产出产物。
 */
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const platform = (process.env.TAURI_ENV_PLATFORM ?? "").toLowerCase();
const triple = (process.env.TAURI_ENV_TARGET_TRIPLE ?? "").toLowerCase();
const mobileHints = ["android", "ios"];
const isMobile = mobileHints.some((hint) => platform.includes(hint) || triple.includes(hint));

if (isMobile) {
  console.error(
    `[stage-plugins] 跳过：目标平台 \`${platform || triple}\` 是移动端，桌面 worker 产物不适用`,
  );
  process.exit(0);
}

// `tauri build --debug` 会置 TAURI_ENV_DEBUG=1；没有它即 release 打包。
const release = process.env.TAURI_ENV_DEBUG !== "1" && process.env.TAURI_ENV_DEBUG !== "true";
const outDir = path.join(repoRoot, "src-tauri", "plugins", "bundled", "stock");
const args = ["scripts/build-plugin-bundle.mjs", "--out", outDir];
if (release) {
  args.push("--release");
}

console.error(`[stage-plugins] 桌面构建 ⇒ 暂存插件产物（${release ? "release" : "debug"}）`);
const result = spawnSync(process.execPath, args, { stdio: "inherit", cwd: repoRoot });
process.exit(result.status ?? 1);
