#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 按需暂存插件 worker 产物。
 *
 * 这不再是 `tauri build` 的默认前置：`beforeBuildCommand` 只构建前端，`bundle.resources`
 * 也不再包含 `plugins`。插件产物必须由 `npm run plugin:bundle:stage` 或各插件发布流程
 * 显式生成，用户按需安装；默认安装包不携带 17 个插件副本。
 *
 * 缺省落点仍是 `src-tauri/plugins/bundled/<名字>`，供显式本地暂存/迁移脚本使用；
 * 开发期可用 `--root axagent-plugins/crates/plugins/bundled` 落到 `tauri dev` 的源码树路径。
 *
 * 多插件清单来自 `./lib/plugin-repos.mjs`，这里只负责逐插件转调 `build-plugin-bundle.mjs`。
 *
 * `--debug`：手工暂存时用 debug 产物（快）；缺省是 release。
 */
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { ensurePluginBuildRoots } from "./lib/plugin-build-roots.mjs";
import { PLUGIN_NAMES } from "./lib/plugin-repos.mjs";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

/** 取 `--root <值>`；缺省是发布落点。 */
function argRoot() {
  const argv = process.argv.slice(2);
  const idx = argv.indexOf("--root");
  if (idx === -1) {
    return path.join(repoRoot, "src-tauri", "plugins", "bundled");
  }
  const value = argv[idx + 1];
  if (!value || value.startsWith("--")) {
    console.error("[stage-plugins] --root 需要跟一个目录");
    process.exit(2);
  }
  return path.resolve(repoRoot, value);
}

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

// 按需暂存的插件清单：顺序即构建顺序。源码权威是主仓外的独立插件仓
// (`../AxInvest-plugin-repos/<plugin>`，`AXAGENT_PLUGIN_REPOS_ROOT` 可覆盖)。
const PLUGINS = PLUGIN_NAMES;

// `tauri build --debug` 会置 TAURI_ENV_DEBUG=1；没有它即 release 打包。手工直跑要 debug 档
// 用 `--debug`（`npm run plugin:bundle:stage|:dev` 走的就是这条）。
const release =
  process.env.TAURI_ENV_DEBUG !== "1"
  && process.env.TAURI_ENV_DEBUG !== "true"
  && !process.argv.includes("--debug");

const bundledRoot = argRoot();

console.error(
  `[stage-plugins] 桌面构建 ⇒ 暂存 ${PLUGINS.length} 个插件产物到 ${bundledRoot}（${release ? "release" : "debug"}）`,
);
// 独立仓本身就是可 `cargo build -p` 的工作区根（自带 SDK 快照与根 Cargo.toml），
// 直接作为 `--build-root` 传入；不再从主仓模板源物化临时副本。
const buildRoots = ensurePluginBuildRoots(repoRoot, PLUGINS);
for (const plugin of PLUGINS) {
  const outDir = path.join(bundledRoot, plugin);
  // 打包脚本与各独立仓 scripts/ 下的 vendored 副本同源（生成器同步），这里用主仓这份。
  const args = [
    "axagent-plugins/scripts/build-plugin-bundle.mjs",
    "--plugin", plugin,
    "--build-root", buildRoots.get(plugin),
    "--out", outDir,
  ];
  if (release) {
    args.push("--release");
  }
  // release 的 strip/debug 由**发起构建的根**决定：各独立仓与 axagent-plugins 的
  // Cargo.toml 已写 [profile.release]，这里再注入环境变量兜住尚未同步 profile 的仓，
  // 避免某个 worker 漏去符号就把安装包体积抬回 156M。
  const result = spawnSync(process.execPath, args, {
    stdio: "inherit",
    cwd: repoRoot,
    env: {
      ...process.env,
      // 17 个独立仓各自带 SDK/target 会重复编译同一批依赖；显式共享 target-dir。
      // 各仓的 toolchain/profile/rustflags 已同源自检，缓存可安全命中。
      CARGO_TARGET_DIR:
        process.env.AXAGENT_SHARED_TARGET_DIR
        ?? path.join(process.env.HOME ?? repoRoot, ".cargo", "target-axagent"),
      CARGO_PROFILE_RELEASE_STRIP: "symbols",
      CARGO_PROFILE_RELEASE_DEBUG: "0",
    },
  });
  if (result.error) {
    console.error(`[stage-plugins] 启动 ${plugin} 打包失败：${result.error.message}`);
    process.exit(1);
  }
  if (result.status !== 0) {
    console.error(`[stage-plugins] 插件 ${plugin} 打包退出码 ${result.status}`);
    process.exit(result.status ?? 1);
  }
  console.error(`[stage-plugins] ${plugin} ⇒ ${outDir}`);
}
