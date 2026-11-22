// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 独立插件仓的唯一本地定位入口。
 *
 * 业务插件源码的权威位置是主仓外的 17 个独立 Git 仓，不是 `axagent-plugins/crates`。
 * 主应用仍保留宿主 loader、插件 SDK 与构建工具；开发期 staging、CI 验收和发布前检查
 * 都通过本模块找到对应独立仓。默认根可由 `AXAGENT_PLUGIN_REPOS_ROOT` 覆盖。
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const defaultFs = fs;

// 插件名集合从统一契约派生（2026-11-22）。此前这里手抄 17 个名字，与生成器
// WORKER_SPECS、打包器 PLUGIN_SPECS 各持一份；任一方漂移都让「数量对账」全绿而语义分叉。
// checker 仍会把本导出与契约逐项对账，防止有人在这里重新写死一份覆盖值。
const pluginContract = JSON.parse(
  fs.readFileSync(path.join(path.dirname(fileURLToPath(import.meta.url)), "plugin-contract.json"), "utf8"),
);

export const PLUGIN_CONTRACT = Object.freeze(pluginContract);
export const PLUGIN_NAMES = Object.freeze(pluginContract.plugins.map((p) => p.name));
export const WORKER_PLUGIN_NAMES = Object.freeze(
  pluginContract.plugins.filter((p) => p.strategy === "worker").map((p) => p.name),
);
export const DOMAIN_PACK_PLUGINS = Object.freeze(
  pluginContract.plugins.filter((p) => p.domainPackId),
);

export function pluginReposRoot(repoRoot) {
  return path.resolve(
    process.env.AXAGENT_PLUGIN_REPOS_ROOT
      ?? path.join(path.dirname(repoRoot), "AxInvest-plugin-repos"),
  );
}

export function pluginRepo(repoRoot, plugin) {
  return path.join(pluginReposRoot(repoRoot), plugin);
}

export function pluginRepoMetadata(repoRoot, plugin, reposRoot) {
  const repo = reposRoot ? path.join(reposRoot, plugin) : pluginRepo(repoRoot, plugin);
  return path.join(repo, ".axinvest-plugin-repos.json");
}

/** 读取独立仓持久化的发布坐标；旧仓无元数据时返回 null。 */
export function readPluginRepoMetadata(repoRoot, plugin, fs = defaultFs, reposRoot) {
  const metadataPath = pluginRepoMetadata(repoRoot, plugin, reposRoot);
  if (!fs.existsSync(metadataPath)) return null;
  const value = JSON.parse(fs.readFileSync(metadataPath, "utf8"));
  if (
    typeof value?.owner !== "string" || !value.owner.trim()
    || typeof value?.repo !== "string" || !value.repo.trim()
    || typeof value?.plugin !== "string" || value.plugin !== plugin
    || typeof value?.releaseManifestUrl !== "string"
    || typeof value?.pluginVersion !== "string"
  ) {
    throw new Error(`独立插件仓发布元数据非法：${metadataPath}`);
  }
  return value;
}

/** 独立仓自带的 bundle 命令；主仓只负责转调，不维护第二份插件规格。 */
export function pluginBundleScript(repoRoot, plugin) {
  return path.join(pluginRepo(repoRoot, plugin), "scripts", "build-plugin-bundle.mjs");
}

export function pluginBundleDist(repoRoot, plugin) {
  return path.join(pluginRepo(repoRoot, plugin), "dist", "plugins", plugin);
}

/**
 * 域包源码位置。`plugin-opc` 是公共包，其余独立仓的目录形如
 * `<repo>/plugin-opc-accounting/domain_packs`；声明式插件没有 Cargo 目录，但目录名相同。
 */
export function pluginDomainPacksDir(repoRoot, pluginId) {
  const plugin = pluginId.replace(/^plugin-/, "");
  return path.join(pluginRepo(repoRoot, plugin), `plugin-${plugin}`, "domain_packs");
}

export function pluginWebPagesDir(repoRoot, plugin) {
  return path.join(pluginRepo(repoRoot, plugin), `plugin-${plugin}`, "web", "pages");
}

export function pluginInstalledDir(repoRoot, plugin) {
  return path.join(repoRoot, "src-tauri", "plugins", "bundled", plugin);
}

/**
 * 声明式域 E2E 要求所有 bundle 共处一个根。独立仓的 dist 各自分开，
 * nextest 前置把构建产物链接/复制到该临时聚合根，测试只读聚合结果。
 */
export function declarativeBundleMatrixRoot(repoRoot) {
  return path.join(repoRoot, ".tmp", "plugin-bundles");
}

/** 校验独立仓与当前插件名一一对应，防止拼错路径后静默扫描空目录。 */
export function assertPluginRepo(repoRoot, plugin) {
  const repo = pluginRepo(repoRoot, plugin);
  const script = pluginBundleScript(repoRoot, plugin);
  if (!fs.existsSync(repo) || !fs.existsSync(path.join(repo, ".git"))) {
    throw new Error(
      `独立插件仓不存在或不是 Git 仓：${repo}（可用 AXAGENT_PLUGIN_REPOS_ROOT 覆盖根目录）`,
    );
  }
  if (!fs.existsSync(script)) {
    throw new Error(`独立插件仓缺少 bundle 脚本：${script}`);
  }
  return { repo, script };
}

export function assertAllPluginRepos(repoRoot, plugins = PLUGIN_NAMES) {
  return plugins.map((plugin) => ({ plugin, ...assertPluginRepo(repoRoot, plugin) }));
}

export function listPluginRepos(repoRoot) {
  const root = pluginReposRoot(repoRoot);
  return fs.existsSync(root)
    ? fs.readdirSync(root, { withFileTypes: true })
      .filter((entry) => entry.isDirectory() && fs.existsSync(path.join(root, entry.name, ".git")))
      .map((entry) => entry.name)
      .sort()
    : [];
}

