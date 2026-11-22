// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 插件 worker 的**临时构建根**。
 *
 * 可交付插件源码的权威在 17 个独立 Git 仓，但它们只由各插件仓的 Release job checkout；
 * 主仓 CI / Release 只 checkout 本仓，不能强依赖 sibling repos（否则整条链当场红）。
 * 主仓 `axagent-plugins/crates/plugin-*` 是生成模板源，被 `exclude` 隔在主 workspace 外，
 * 直接 `cargo build -p <packageName>` 会报 "package ID specification did not match"。
 *
 * 本模块用**已有的生成器**从主仓模板源物化出与独立仓同构的临时工作区（含 SDK 快照与
 * 根 Cargo.toml），只用作 cargo 构建根；打包、页面资产、清单仍由主仓脚本负责。
 * 这样 CI 里的构建输入与已发布的独立仓一致，又不引入对仓外目录的强依赖。
 */
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { PLUGIN_NAMES } from "./plugin-repos.mjs";

const GENERATOR = "scripts/generate-plugin-independent-repos.mjs";

export function pluginBuildRootsDir(repoRoot) {
  return path.resolve(
    process.env.AXAGENT_PLUGIN_BUILD_ROOTS_DIR ?? path.join(repoRoot, "target", "plugin-build-roots"),
  );
}

export function pluginBuildRoot(repoRoot, plugin) {
  return path.join(pluginBuildRootsDir(repoRoot), plugin);
}

/**
 * 逐插件补齐临时构建根（已存在的跳过），任一生成失败即中止。
 * 返回所有请求的构建根，供调用方 fail-closed 校验。
 */
export function ensurePluginBuildRoots(repoRoot, plugins = PLUGIN_NAMES) {
  const rootsDir = pluginBuildRootsDir(repoRoot);
  fs.mkdirSync(rootsDir, { recursive: true });
  const roots = new Map();
  for (const plugin of plugins) {
    const root = pluginBuildRoot(repoRoot, plugin);
    if (!fs.existsSync(path.join(root, "Cargo.toml"))) {
      const result = spawnSync(
        process.execPath,
        [GENERATOR, "--bootstrap-from-main", "--plugin", plugin, "--skip-git", "--out", rootsDir],
        { cwd: repoRoot, stdio: "inherit", windowsHide: true },
      );
      if (result.error) {
        throw new Error(`物化 ${plugin} 插件构建根失败：${result.error.message}`);
      }
      if (result.status !== 0) {
        throw new Error(`物化 ${plugin} 插件构建根失败（exit ${result.status}）`);
      }
    }
    roots.set(plugin, root);
  }
  const missing = [...roots].filter(([, root]) => !fs.existsSync(path.join(root, "Cargo.toml")))
    .map(([plugin]) => plugin);
  if (missing.length) {
    throw new Error(`插件构建根缺少 Cargo.toml：${missing.join(", ")}（根：${rootsDir}）`);
  }
  return roots;
}
