#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 主仓侧插件打包入口。
 *
 * 业务插件源码的权威位置在 17 个外置 Git 仓。本脚本只解析 `--plugin`，然后
 * 在对应独立仓执行该仓自带的 `scripts/build-plugin-bundle.mjs`，避免主仓维护
 * 第二份插件规格或回退读取即将删除的 `axagent-plugins/crates`。
 *
 * 用法：
 *   npm run plugin:bundle -- --plugin stock --check
 *   npm run plugin:bundle -- --plugin wiki --release --out /tmp/wiki
 *   node scripts/run-plugin-bundle.mjs --all --check
 */
import { spawnSync } from "node:child_process";
import path from "node:path";
import { PLUGIN_NAMES, assertPluginRepo, pluginRepo } from "./lib/plugin-repos.mjs";

const repoRoot = path.resolve(import.meta.dirname, "..");
const argv = process.argv.slice(2);
const pluginFlag = argv.indexOf("--plugin");
const explicitPlugin = pluginFlag >= 0 ? argv[pluginFlag + 1] : null;
if (pluginFlag >= 0 && (!explicitPlugin || explicitPlugin.startsWith("--"))) {
  console.error("[plugin-bundle] --plugin 需要跟一个插件名");
  process.exit(2);
}

if (explicitPlugin && explicitPlugin !== "all" && !PLUGIN_NAMES.includes(explicitPlugin)) {
  console.error(`[plugin-bundle] 未知插件：${explicitPlugin}`);
  console.error(`[plugin-bundle] 可选：${PLUGIN_NAMES.join(" | ")} | all`);
  process.exit(2);
}

const all = argv.includes("--all") || explicitPlugin === "all";
const plugins = all ? PLUGIN_NAMES : [explicitPlugin ?? "stock"];
const passthrough = argv.filter((arg, index) => {
  if (arg === "--all") return false;
  if (arg === "--plugin") return false;
  if (pluginFlag >= 0 && index === pluginFlag + 1) return false;
  return true;
});

for (const plugin of plugins) {
  const { repo, script } = assertPluginRepo(repoRoot, plugin);
  const result = spawnSync(
    process.execPath,
    [script, "--plugin", plugin, ...passthrough],
    { cwd: repo, stdio: "inherit", env: process.env },
  );
  if (result.error) {
    console.error(`[plugin-bundle] 启动 ${plugin} 打包失败：${result.error.message}`);
    process.exit(1);
  }
  if (result.status !== 0) {
    console.error(`[plugin-bundle] ${plugin} 打包退出码 ${result.status}（仓：${pluginRepo(repoRoot, plugin)}）`);
    process.exit(result.status ?? 1);
  }
}

console.log(`[plugin-bundle] ${plugins.length}/${plugins.length} 个外置插件仓打包通过`);
