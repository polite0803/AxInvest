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
 * 而 `cargo build -p <worker>` 编出的是**宿主架构**的可执行文件 —— 打进移动端
 * 包里毫无用处（还白增体积）。因此只在桌面平台暂存；移动端直接跳过并退 0。
 *
 * 识别依据是 Tauri CLI 注入的环境变量（`TAURI_ENV_PLATFORM` / `TAURI_ENV_TARGET_TRIPLE`）。
 * 两个都没有时（手工直跑本脚本）按「桌面」处理 —— 手工运行本来就是想产出产物。
 *
 * 多插件（任务 #36 第 1 批）：本脚本只负责**逐插件**转调 `build-plugin-bundle.mjs`，
 * 插件清单与各自产物形状全在那个脚本的 `PLUGIN_SPECS` 里，这里不复制第二份。
 * 新增插件 = 下方 `PLUGINS` 数组加一个名字。
 *
 * 落点（任务 #163）：缺省 `src-tauri/plugins/bundled/<名字>`（发布打包用，与
 * `bundle.resources` 的配置目录对齐）。开发期宿主读的是**编译期默认根**
 * `axagent-plugins/crates/plugins/bundled/`（见 `PluginManagerConfig::resolve_bundled_root`），
 * 用 `--root <目录>` 指过去。此前 `package.json` 的两条 `plugin:bundle:*` 脚本把 `--out`
 * 写死成 `.../stock` —— 那是本清单的**第二份**，新增插件时不会有人想起改它，结果开发期
 * 盘上根本没有 opc 产物、插件解析不到，功能静默下线。⇒ 两条脚本一律转调本脚本。
 *
 * `--debug`：本地脚本用的档。发布路径靠 Tauri 注入的 `TAURI_ENV_DEBUG` 判档，手工直跑时
 * 缺省是 release；开发期要的是 debug 产物（快、且与 `tauri dev` 同档），故显式给。
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

// 暂存的插件清单：顺序即构建顺序。这里打的是**宿主安装包**，源码权威是主仓外的
// 17 个独立插件仓（`../AxInvest-plugin-repos/<plugin>`，`AXAGENT_PLUGIN_REPOS_ROOT` 可覆盖）。
// 主仓 `axagent-plugins/crates/plugin-*` 副本不再参与构建；CI / Release 必须先获取这 17 个仓
// （取源方案尚未定，缺仓时 `ensurePluginBuildRoots` 直接报错而不是回退主仓副本）。
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
  const result = spawnSync(process.execPath, args, { stdio: "inherit", cwd: repoRoot });
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
