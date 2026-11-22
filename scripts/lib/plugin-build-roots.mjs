// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 插件 worker 的**构建根解析**。
 *
 * 可交付插件源码的权威是 17 个独立 Git 仓（`../AxInvest-plugin-repos/<plugin>`，
 * `AXAGENT_PLUGIN_REPOS_ROOT` 可覆盖）。每个独立仓自带 SDK 快照与根 `Cargo.toml`，
 * 本身就是可 `cargo build -p <packageName>` 的工作区根 ⇒ 不需要再物化任何临时副本。
 *
 * 2026-11-22 变更（物理删除前置条件）：此前这里调用生成器的「从主仓模板物化」引导模式，
 * 从主仓 `axagent-plugins/crates/plugin-*` 模板源物化临时构建根 ⇒ 主仓副本既是「待删的」
 * 又是「构建必需的」，删除永远无法落地。该引导调用已整体移除：现在直接返回独立仓根，
 * 并对每个根 fail-closed 校验（Git 仓 + 根 Cargo.toml）。主仓副本不再参与构建。
 *
 * 代价（已知并接受）：CI / Release 必须在构建前获取 17 个独立仓。缺仓时这里直接报错，
 * **不退回主仓副本** —— 回退会让「独立仓是唯一源」再次变成口号（取源方案见
 * `check-plugin-source-ownership.mjs` 的第 5 项前置，尚未实施）。
 */
import fs from "node:fs";
import path from "node:path";
import { PLUGIN_NAMES, pluginRepo, pluginReposRoot } from "./plugin-repos.mjs";

/** 独立仓根（必须在主仓之外）—— 插件源码与 cargo 工作区的唯一权威。 */
export function pluginSourceRootsDir(repoRoot) {
  return pluginReposRoot(repoRoot);
}

/** 单个插件的 cargo 构建根 = 其独立仓根。 */
export function pluginBuildRoot(repoRoot, plugin) {
  return pluginRepo(repoRoot, plugin);
}

/**
 * 逐插件校验构建根可用，任一不可用即中止（fail-closed）。
 * 返回 `Map<plugin, root>`，供调用方直接传给 `--build-root`。
 */
export function ensurePluginBuildRoots(repoRoot, plugins = PLUGIN_NAMES) {
  const roots = new Map();
  const problems = [];
  for (const plugin of plugins) {
    const root = pluginBuildRoot(repoRoot, plugin);
    if (!fs.existsSync(path.join(root, ".git"))) {
      problems.push(`${plugin}: 不是 Git 仓（${root}）`);
    } else if (!fs.existsSync(path.join(root, "Cargo.toml"))) {
      problems.push(`${plugin}: 缺少根 Cargo.toml（${root}）`);
    }
    roots.set(plugin, root);
  }
  if (problems.length) {
    throw new Error(
      `插件构建根不可用（独立仓是唯一权威，不回退主仓副本）：${problems.join("；")}`,
    );
  }
  return roots;
}
