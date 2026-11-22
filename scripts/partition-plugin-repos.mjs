#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 将业务插件源码的权威位置从主仓迁到 17 个独立仓。
 *
 * 默认只做逐文件审计，不写工作树。显式 `--apply` 才会：
 *   1. 删除主仓 17 个业务插件目录与 stock-core；
 *   2. 把 axagent-plugins 的 workspace 收缩为宿主 loader、SDK 与 schema-gen；
 *   3. 再次核对独立仓已承接全部可迁移源码，并验证主仓不再含业务插件 crate。
 *
 * 这是单仓迁移工具，不执行 git remote / push / release。
 * 独立仓根默认 `../AxInvest-plugin-repos`，可用 AXAGENT_PLUGIN_REPOS_ROOT 覆盖。
 *
 * 用法：
 *   node scripts/partition-plugin-repos.mjs
 *   node scripts/partition-plugin-repos.mjs --json
 *   node scripts/partition-plugin-repos.mjs --apply
 */
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { PLUGIN_NAMES, pluginRepo, pluginReposRoot } from "./lib/plugin-repos.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const HOST_ROOT = join(ROOT, "axagent-plugins");
const CRATES_ROOT = join(HOST_ROOT, "crates");
const JSON_OUT = process.argv.includes("--json");
const APPLY = process.argv.includes("--apply");
const KEPT_CRATES = new Set(["plugin-opc-core", "plugin-testkit", "plugins", "schema-gen"]);
const BUSINESS_CRATES = [
  ...PLUGIN_NAMES.map((plugin) => `plugin-${plugin}`),
  "stock-core",
];

const fail = (message) => {
  console.error(`FAIL: ${message}`);
  process.exit(1);
};

function walkFiles(root, options = {}) {
  if (!existsSync(root)) return [];
  const files = [];
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const full = join(root, entry.name);
    const rel = relative(root, full);
    if (entry.isDirectory()) {
      if (options.skip?.has(entry.name)) continue;
      files.push(...walkFiles(full, options).map((file) => join(entry.name, file)));
    } else if (entry.isFile()) {
      files.push(rel);
    }
  }
  return files.sort();
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function compareTree(source, target) {
  const skip = new Set(["target", "dist", "node_modules", ".git"]);
  // Cargo.toml 在独立仓会被改写为仓内 path 依赖，单独比较 package 名与版本，
  // 不把生成器的预期差异误报为源码漂移。
  const sourceFiles = walkFiles(source, { skip }).filter((file) => file !== "Cargo.toml");
  const targetFiles = walkFiles(target, { skip }).filter((file) => file !== "Cargo.toml");
  const sourceSet = new Set(sourceFiles);
  const targetSet = new Set(targetFiles);
  const missing = sourceFiles.filter((file) => !targetSet.has(file));
  const extra = targetFiles.filter((file) => !sourceSet.has(file));
  const changed = sourceFiles.filter((file) => {
    if (!targetSet.has(file)) return false;
    return sha256(join(source, file)) !== sha256(join(target, file));
  });
  return { sourceFiles: sourceFiles.length, targetFiles: targetFiles.length, missing, extra, changed };
}

function compareManifest(source, target) {
  if (!existsSync(source) || !existsSync(target)) {
    return {
      sourcePackage: existsSync(source) ? "present" : null,
      targetPackage: existsSync(target) ? "present" : null,
      sourceVersion: null,
      targetVersion: null,
      comparable: false,
    };
  }
  const text = readFileSync(source, "utf8");
  const targetText = readFileSync(target, "utf8");
  const inheritedVersion = text.match(/^version\.workspace\s*=\s*true\s*$/m)
    ? readFileSync(join(HOST_ROOT, "Cargo.toml"), "utf8")
      .match(/^\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m)?.[1] ?? null
    : null;
  return {
    sourcePackage: text.match(/^\[package\][\s\S]*?^name\s*=\s*"([^"]+)"/m)?.[1] ?? null,
    targetPackage: targetText.match(/^\[package\][\s\S]*?^name\s*=\s*"([^"]+)"/m)?.[1] ?? null,
    sourceVersion: text.match(/^\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m)?.[1] ?? inheritedVersion,
    targetVersion: targetText.match(/^\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m)?.[1] ?? null,
    comparable: true,
  };
}

function validateWorkspaceMembers() {
  const manifest = readFileSync(join(HOST_ROOT, "Cargo.toml"), "utf8");
  const block = manifest.match(/\[workspace\][\s\S]*?members\s*=\s*\[([\s\S]*?)\]/)?.[1];
  if (!block) fail("无法解析 axagent-plugins workspace members");
  return [...block.matchAll(/"crates\/([^"]+)"/g)].map((match) => match[1]);
}

function expectedMembersAfterPartition() {
  return [...KEPT_CRATES].sort();
}

function validateTargets() {
  const checks = [];
  for (const plugin of PLUGIN_NAMES) {
    const repo = pluginRepo(ROOT, plugin);
    if (!existsSync(join(repo, ".git"))) fail(`外置仓缺失或不是 Git 仓：${repo}`);
    const source = join(CRATES_ROOT, `plugin-${plugin}`);
    const target = join(repo, `plugin-${plugin}`);
    if (existsSync(source) && existsSync(target)) {
      const tree = compareTree(source, target);
      const manifest = compareManifest(
        join(source, "Cargo.toml"),
        join(target, "Cargo.toml"),
      );
      const cargoMatches = !manifest.comparable
        || (manifest.sourcePackage === manifest.targetPackage
          && manifest.sourceVersion === manifest.targetVersion);
      checks.push({
        plugin,
        source: relative(ROOT, source),
        target: relative(ROOT, target),
        tree,
        manifest,
        cargoMatches,
        ok: cargoMatches
          && tree.missing.length === 0
          && tree.changed.length === 0
          && tree.extra.length === 0,
      });
    }
  }
  const stockSource = join(CRATES_ROOT, "stock-core");
  const stockTarget = join(pluginRepo(ROOT, "stock"), "stock-core");
  if (existsSync(stockSource) && existsSync(stockTarget)) {
    const tree = compareTree(stockSource, stockTarget);
    const manifest = compareManifest(
      join(stockSource, "Cargo.toml"),
      join(stockTarget, "Cargo.toml"),
    );
    const cargoMatches = !manifest.comparable
      || (manifest.sourcePackage === manifest.targetPackage
        && manifest.sourceVersion === manifest.targetVersion);
    checks.push({
      plugin: "stock-core",
      source: relative(ROOT, stockSource),
      target: relative(ROOT, stockTarget),
      tree,
      manifest,
      cargoMatches,
      ok: cargoMatches
        && tree.missing.length === 0
        && tree.changed.length === 0
        && tree.extra.length === 0,
    });
  }
  return checks;
}

function runBundleChecks() {
  const failures = [];
  for (const plugin of PLUGIN_NAMES) {
    const result = spawnSync(
      process.execPath,
      [join(pluginRepo(ROOT, plugin), "scripts", "build-plugin-bundle.mjs"), "--plugin", plugin, "--check"],
      {
        cwd: pluginRepo(ROOT, plugin),
        encoding: "utf8",
        env: { ...process.env, CARGO_INCREMENTAL: "0" },
      },
    );
    if (result.status !== 0) {
      failures.push({
        plugin,
        status: result.status,
        detail: `${result.stdout ?? ""}${result.stderr ?? ""}`.trim().split("\n").slice(-8).join("\n"),
      });
    }
  }
  return failures;
}

function rewriteWorkspace() {
  const path = join(HOST_ROOT, "Cargo.toml");
  const text = readFileSync(path, "utf8");
  const members = expectedMembersAfterPartition()
    .map((member) => `  "crates/${member}",`)
    .join("\n");
  const next = text.replace(
    /members\s*=\s*\[[\s\S]*?\]/,
    `members = [\n${members}\n]`,
  );
  if (next === text) fail("axagent-plugins workspace members 未发生变化");
  writeFileSync(path, next);
}

function main() {
  if (!existsSync(HOST_ROOT)) fail(`主仓插件目录不存在：${HOST_ROOT}`);
  const repositoriesRoot = pluginReposRoot(ROOT);
  if (repositoriesRoot === HOST_ROOT || repositoriesRoot.startsWith(`${ROOT}${sep}`)) {
    // 外置仓必须在主仓之外，否则删除业务目录会碰到独立仓的源码。
    fail(`外置插件仓根必须位于主仓之外：${repositoriesRoot}`);
  }
  const workspaceMembers = validateWorkspaceMembers();
  const checks = validateTargets();
  const failedChecks = checks.filter((check) => !check.ok);
  const remainingBusinessDirs = BUSINESS_CRATES
    .filter((name) => existsSync(join(CRATES_ROOT, name)))
    .sort();

  if (APPLY) {
    if (failedChecks.length > 0) {
      fail(`独立仓尚未完整承接源码，拒绝 --apply：${failedChecks.map((c) => c.plugin).join("、")}`);
    }
    for (const name of remainingBusinessDirs) {
      rmSync(join(CRATES_ROOT, name), { recursive: true, force: true });
    }
    rewriteWorkspace();
  }

  const finalWorkspaceMembers = validateWorkspaceMembers();
  const finalBusinessDirs = BUSINESS_CRATES.filter((name) => existsSync(join(CRATES_ROOT, name)));
  const expectedMembers = expectedMembersAfterPartition();
  const workspaceOk = JSON.stringify([...finalWorkspaceMembers].sort()) === JSON.stringify(expectedMembers);
  const physicalOk = finalBusinessDirs.length === 0;
  const bundleFailures = APPLY ? runBundleChecks() : [];

  const result = {
    mode: APPLY ? "apply" : "audit",
    repositoriesRoot,
    workspaceBefore: workspaceMembers,
    workspaceAfter: finalWorkspaceMembers,
    expectedWorkspaceMembers: expectedMembers,
    businessCratesBefore: remainingBusinessDirs,
    businessCratesAfter: finalBusinessDirs,
    repositoryChecks: checks.map((check) => ({
      plugin: check.plugin,
      ok: check.ok,
      sourceFiles: check.tree.sourceFiles,
      targetFiles: check.tree.targetFiles,
      missing: check.tree.missing,
      extra: check.tree.extra,
      changed: check.tree.changed,
      manifest: check.manifest,
    })),
    bundleFailures,
    ok: failedChecks.length === 0 && workspaceOk && physicalOk && bundleFailures.length === 0,
  };

  if (JSON_OUT) {
    console.log(JSON.stringify(result, null, 2));
  } else {
    console.log(`模式：${result.mode}`);
    console.log(`外置仓根：${repositoriesRoot}`);
    console.log(`workspace：${workspaceMembers.join(", ")} -> ${finalWorkspaceMembers.join(", ")}`);
    console.log(`主仓业务插件目录：${remainingBusinessDirs.length} -> ${finalBusinessDirs.length}`);
    for (const check of result.repositoryChecks) {
      console.log(
        `${check.ok ? "PASS" : "FAIL"} ${check.plugin}: ${check.sourceFiles} files -> ${check.targetFiles}`
          + `，缺失 ${check.missing.length} / 多出 ${check.extra.length} / 改动 ${check.changed.length}`,
      );
      if (check.missing.length) console.log(`  缺失：${check.missing.join(", ")}`);
      if (check.extra.length) console.log(`  多出：${check.extra.join(", ")}`);
      if (check.changed.length) console.log(`  改动：${check.changed.join(", ")}`);
    }
    if (bundleFailures.length) {
      for (const failure of bundleFailures) {
        console.log(`FAIL bundle ${failure.plugin} (exit ${failure.status})\n${failure.detail}`);
      }
    }
    console.log(result.ok ? (APPLY ? "PLUGIN_REPOS_PARTITION_OK" : "PLUGIN_REPOS_PARTITION_AUDIT_OK") : "PLUGIN_REPOS_PARTITION_FAIL");
  }
  if (!result.ok) process.exit(1);
}

main();
