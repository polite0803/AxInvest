#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * SDK 与独立插件仓的真实发布演练。
 *
 * 目的：同时验证 SDK 发布形状与独立插件仓自包含：
 *   1. 按依赖顺序执行 `cargo package`，生成并验证 `.crate`；
 *   2. 解包发布物，确认归一化 manifest 中不再泄露仓内 path；
 *   3. 暂存并在临时副本中验证现有 17 个独立仓，确认每仓携带最小 SDK 且依赖不越出仓根；
 *   4. 不注入主仓路径或临时 registry patch，直接完成 17/17 bundle 构建。
 *
 * `cargo package` 只证明 SDK 发布物形状；插件仓构建并不消费这些 `.crate`。
 * 这不冒充已经执行 `cargo publish` 或已推送远端。
 *
 * 用法：
 *   node scripts/verify-plugin-sdk-release-rehearsal.mjs
 *   node scripts/verify-plugin-sdk-release-rehearsal.mjs --keep
 */
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { PLUGIN_NAMES, pluginRepo, pluginReposRoot } from "./lib/plugin-repos.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const KEEP = process.argv.includes("--keep");
const PACKAGE_TARGET = mkdtempSync(join(tmpdir(), "axinvest-sdk-package-"));
const REHEARSAL_ROOT = mkdtempSync(join(tmpdir(), "axinvest-plugin-release-"));
const SDK_PACKAGES = [
  {
    name: "axagent-plugin-proto",
    manifest: "axagent-core/crates/plugin-proto/Cargo.toml",
    dependencies: [],
  },
  {
    name: "axagent-harness",
    manifest: "axagent-core/crates/harness/Cargo.toml",
    dependencies: [],
  },
  {
    name: "axagent-plugin-opc-core",
    manifest: "axagent-plugins/crates/plugin-opc-core/Cargo.toml",
    dependencies: ["axagent-plugin-proto"],
  },
  {
    name: "axagent-plugin-testkit",
    manifest: "axagent-plugins/crates/plugin-testkit/Cargo.toml",
    dependencies: ["axagent-plugin-proto"],
  },
];

function fail(message) {
  console.error(`FAIL: ${message}`);
  process.exit(1);
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: ROOT,
    encoding: "utf8",
    windowsHide: true,
    ...options,
  });
  const output = `${result.stdout ?? ""}${result.stderr ?? ""}`.trim();
  if (result.error || result.status !== 0) {
    fail(
      `${command} ${args.join(" ")} 失败：${result.error?.message ?? `exit ${result.status}`}\n${output}`,
    );
  }
  return output;
}

function packageField(text, field) {
  const block = text.match(/^\[package\]([\s\S]*?)(?=^\[)/m)?.[1] ?? "";
  return block.match(new RegExp(`^${field}\\s*=\\s*"([^"]+)"`, "m"))?.[1] ?? null;
}

function patchConfig(packages) {
  const entries = packages
    .map((item) => `${item.name} = { path = ${JSON.stringify(item.extracted)} }`)
    .join("\n");
  return `[patch.crates-io]\n${entries}\n`;
}

function packageSdk(spec, available) {
  const manifestPath = join(ROOT, spec.manifest);
  const manifest = readFileSync(manifestPath, "utf8");
  const version = packageField(manifest, "version") ?? "3.0.0";
  const configPath = join(REHEARSAL_ROOT, `${spec.name}.config.toml`);
  writeFileSync(configPath, patchConfig(available));
  run("cargo", [
    "package",
    "--allow-dirty",
    "--manifest-path",
    manifestPath,
    "--target-dir",
    PACKAGE_TARGET,
    "--config",
    configPath,
  ]);

  const crate = join(PACKAGE_TARGET, "package", `${spec.name}-${version}.crate`);
  if (!existsSync(crate)) {
    fail(`cargo package 未生成 ${relative(ROOT, crate)}`);
  }
  const extractRoot = join(REHEARSAL_ROOT, "packages");
  mkdirSync(extractRoot, { recursive: true });
  run("tar", ["-xzf", crate, "-C", extractRoot]);
  const extracted = join(extractRoot, `${spec.name}-${version}`);
  const packagedManifest = join(extracted, "Cargo.toml");
  if (!existsSync(packagedManifest)) {
    fail(`${spec.name} 解包后缺少 Cargo.toml`);
  }
  const normalized = readFileSync(packagedManifest, "utf8");
  for (const dependency of spec.dependencies) {
    if (new RegExp(`^\\[dependencies\\.${dependency.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\]$`, "m").test(normalized)) {
      continue;
    }
    const dependencyBlock = new RegExp(
      `\\[dependencies\\.${dependency.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\]([\\s\\S]*?)(?=^\\[|\\s*$)`,
      "m",
    ).exec(normalized)?.[1];
    if (!dependencyBlock || !/^version\s*=/m.test(dependencyBlock)) {
      fail(`${spec.name} 的发布物缺少 ${dependency} 版本依赖`);
    }
    if (/^path\s*=/m.test(dependencyBlock)) {
      fail(`${spec.name} 的发布物仍含 ${dependency} path 依赖`);
    }
  }
  if (/path\s*=\s*"(?:\.\.\/|.*axagent-(?:core|plugins))/m.test(normalized)) {
    fail(`${spec.name} 的发布物仍泄露仓内 path 依赖`);
  }
  const digest = createHash("sha256").update(readFileSync(crate)).digest("hex");
  return { ...spec, version, crate, extracted, digest };
}

function listManifestFiles(root) {
  const result = [];
  function visit(dir) {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const full = join(dir, entry.name);
      if (entry.isDirectory()) {
        visit(full);
      } else if (entry.isFile() && entry.name === "Cargo.toml") {
        result.push(full);
      }
    }
  }
  visit(root);
  return result;
}

function findSelfContainedViolations(reposRoot) {
  const violations = [];
  for (const repo of readdirSync(reposRoot, { withFileTypes: true })) {
    if (!repo.isDirectory()) { continue; }
    const repoRoot = join(reposRoot, repo.name);
    const rootText = existsSync(join(repoRoot, "Cargo.toml"))
      ? readFileSync(join(repoRoot, "Cargo.toml"), "utf8")
      : "";
    if (/axagent-plugin-/.test(rootText) && !existsSync(join(repoRoot, "sdk", "Cargo.toml"))) {
      violations.push(`${repo.name}: 缺少自包含 SDK 工作区 sdk/Cargo.toml`);
    }
    for (const manifest of listManifestFiles(repoRoot)) {
      const text = readFileSync(manifest, "utf8");
      if (/axagent-(?:plugin-(?:proto|opc-core|testkit)|harness)\s*=\s*"3\.0\.0"/.test(text)) {
        violations.push(`${relative(reposRoot, manifest)}: SDK 仍是外部 registry 版本依赖`);
      }
      if (/path\s*=\s*"[^"]*axagent-(?:core|plugins)\//.test(text)) {
        violations.push(`${relative(reposRoot, manifest)}: 仍指向主仓 axagent 路径`);
      }
      if (/path\s*=\s*"[^"]*AxInvest[^"]*"/.test(text)) {
        violations.push(`${relative(reposRoot, manifest)}: 仍指向主仓绝对路径`);
      }
    }
  }
  return violations;
}

function assertSelfContainedRepos(reposRoot) {
  const violations = findSelfContainedViolations(reposRoot);
  if (violations.length) {
    fail(`独立仓自包含形态不合格：\n${violations.join("\n")}`);
  }
}

function installNodeDependencies(repo) {
  const stockRepo = join(repo, "stock");
  const packageJson = join(stockRepo, "package.json");
  const packageLock = join(stockRepo, "package-lock.json");
  if (!existsSync(packageJson) || !existsSync(packageLock)) {
    fail("stock 独立仓缺少 package.json / package-lock.json，无法验证干净克隆依赖");
  }
  run("npm", ["ci", "--no-audit", "--no-fund"], { cwd: stockRepo });
}

function verifyRepos(reposRoot) {
  const expected = readdirSync(reposRoot, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .filter((entry) => PLUGIN_NAMES.includes(entry.name))
    .map((entry) => entry.name)
    .sort();
  if (expected.length !== 17) {
    fail(`发布演练应生成 17 个仓库，实得 ${expected.length}`);
  }
  const failed = [];
  for (const plugin of expected) {
    const repo = join(reposRoot, plugin);
    const script = join(repo, "scripts", "build-plugin-bundle.mjs");
    const result = spawnSync(process.execPath, [script, "--plugin", plugin, "--check"], {
      cwd: repo,
      encoding: "utf8",
      windowsHide: true,
    });
    if (result.error || result.status !== 0) {
      failed.push({
        plugin,
        detail: `${result.stdout ?? ""}${result.stderr ?? ""}`.trim().split("\n").slice(-12).join("\n"),
      });
    }
  }
  if (failed.length) {
    fail(
      `发布模式 bundle 失败 ${failed.length}/17：\n${
        failed.map((item) => `${item.plugin}\n${item.detail}`).join("\n\n")
      }`,
    );
  }
  return expected;
}

function cleanup() {
  if (KEEP) {
    console.log(`保留 SDK 打包目录：${PACKAGE_TARGET}`);
    console.log(`保留发布演练目录：${REHEARSAL_ROOT}`);
    return;
  }
  for (const dir of [PACKAGE_TARGET, REHEARSAL_ROOT]) {
    if (!existsSync(dir)) { continue; }
    try {
      rmSync(dir, { recursive: true, force: true });
    } catch {
      // Windows 上 cargo 守护句柄可能短暂占用；临时目录清理失败不影响门禁结论。
    }
  }
}

function main() {
  const negativeRoot = join(REHEARSAL_ROOT, "negative-control");
  const negativeRepo = join(negativeRoot, "wiki");
  mkdirSync(negativeRepo, { recursive: true });
  writeFileSync(
    join(negativeRepo, "Cargo.toml"),
    '[package]\nname = "plugin-wiki"\nversion = "3.0.0"\n\n[dependencies]\n' +
      'axagent-plugin-proto = "3.0.0"\n',
  );
  if (findSelfContainedViolations(negativeRoot).length === 0) {
    fail("负控失效：外部 registry 版本依赖的插件仓未被拒绝");
  }

  const packages = [];
  for (const spec of SDK_PACKAGES) {
    packages.push(packageSdk(spec, packages));
  }

  const reposRoot = join(REHEARSAL_ROOT, "repos");
  const sourceRoot = pluginReposRoot(ROOT);
  mkdirSync(reposRoot, { recursive: true });
  for (const plugin of PLUGIN_NAMES) {
    const source = pluginRepo(ROOT, plugin);
    if (!existsSync(join(source, ".git"))) {
      fail(`独立插件仓不存在或不是 Git 仓：${source}`);
    }
    cpSync(source, join(reposRoot, plugin), {
      recursive: true,
      verbatimSymlinks: true,
      filter: (current) => {
        const relativePath = relative(source, current).split("\\").join("/");
        return relativePath !== ".git" && !relativePath.startsWith(".git/")
          && !relativePath.startsWith("target/") && !relativePath.startsWith("dist/")
          && !relativePath.startsWith("node_modules/");
      },
    });
  }
  assertSelfContainedRepos(reposRoot);
  installNodeDependencies(reposRoot);
  const repos = verifyRepos(reposRoot);
  console.log(
    JSON.stringify(
      {
        sdkPackages: packages.map((item) => ({
          name: item.name,
          version: item.version,
          sha256: item.digest,
          normalizedRegistryDependencies: true,
        })),
        independentRepos: `${repos.length}/${repos.length}`,
        manifestSource: "self-contained-sdk",
        registryInjection: false,
        scope: "release rehearsal; no cargo publish or git remote push",
      },
      null,
      2,
    ),
  );
  console.log("PLUGIN_SDK_RELEASE_REHEARSAL_OK");
}

try {
  main();
} finally {
  cleanup();
}
