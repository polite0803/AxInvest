#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * OPC 插件 SDK 独立树验收（PLAN-repo-split §12.4 / P6.2 终局形态验证）。
 *
 * 判据：拷出最小 SDK 树（axagent-plugins + axagent-core 根配置 + harness/plugin-proto）
 * 到临时目录后，OPC 六个 crate 仍可 cargo check / clippy / test 全绿。
 * 该脚本不修改主仓源码，只用于连续验证独立仓形态；退出码 0 = 独立树验收通过。
 *
 * 用法：node scripts/verify-opc-plugin-sdk.mjs [--keep] [--full]
 *  --full：拷出全部插件 workspace（含 quant/ai/infra/kit 最小依赖）并跑 cargo check --workspace。
 */
import { spawnSync } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync, existsSync, copyFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { tmpdir } from "node:os";

const REPO = resolve(import.meta.dirname, "..");
const argv = process.argv.slice(2);
const KEEP = argv.includes("--keep");
const FULL = argv.includes("--full");
const TEMP_ROOT = mkdtempSync(join(tmpdir(), "axagent-sdk-opc-"));
const OUT = {
  tempRoot: TEMP_ROOT,
  steps: [],
  packages: [],
};

function fail(message) {
  console.error(`FAIL: ${message}`);
  process.exit(1);
}

function run(command, args, cwd, label) {
  const result = spawnSync(command, args, { cwd, stdio: "inherit", windowsHide: true });
  OUT.steps.push({ label, command, cwd, exitCode: result.status });
  if (result.status !== 0) fail(`${label}（exit ${result.status}）`);
}

function record(label, exitCode) {
  OUT.steps.push({ label, exitCode });
}

// 1) 拷出最小 SDK 树。
const sdkRoot = join(TEMP_ROOT, "axagent-plugins");
const coreRoot = join(TEMP_ROOT, "axagent-core");
mkdirSync(sdkRoot, { recursive: true });
mkdirSync(coreRoot, { recursive: true });
cpSync(join(REPO, "axagent-plugins"), sdkRoot, { recursive: true, verbatimSymlinks: true });
for (const f of ["clippy.toml", "rustfmt.toml", "rust-toolchain.toml", "Cargo.toml"]) {
  copyFileSync(join(REPO, "axagent-core", f), join(coreRoot, f));
}
for (const c of FULL ? ["harness", "plugin-proto", "entities"] : ["harness", "plugin-proto"]) {
  cpSync(join(REPO, "axagent-core", "crates", c), join(coreRoot, "crates", c), { recursive: true, verbatimSymlinks: true });
}

// 2) 收窄拷出的 axagent-core workspace 成员到最小集；同步收窄插件 workspace 到 OPC 系。
const coreManifest = join(coreRoot, "Cargo.toml");
const coreText = readFileSync(coreManifest, "utf8");
const coreStart = coreText.indexOf("[workspace]");
const coreEnd = coreText.indexOf("[workspace.package]");
const narrowedCore =
  coreText.slice(0, coreStart) +
  "[workspace]\nresolver = \"2\"\nmembers = [\n  \"crates/harness\",\n  \"crates/plugin-proto\",\n]\n\n" +
  coreText.slice(coreEnd);
writeFileSync(coreManifest, narrowedCore);

const pluginManifest = join(sdkRoot, "Cargo.toml");
if (!FULL) {
  writeFileSync(
    pluginManifest,
    readFileSync(pluginManifest, "utf8").replace(/  "crates\/plugin-stock",\n/, ""),
  );
}
if (FULL) {
  // 全插件树需要 stock 依赖链拉出的四个最小 workspace。按最小成员拷出。
  const extraRoots = [
    ["axagent-quant", ["analysis-engine", "astock-data"]],
    ["axagent-ai", ["trajectory"]],
    ["axagent-infra", ["dao"]],
    ["axagent-kit", ["kit"]],
  ];
  for (const [rootName, crates] of extraRoots) {
    const rootDir = join(TEMP_ROOT, rootName);
    mkdirSync(rootDir, { recursive: true });
    for (const f of ["clippy.toml", "rustfmt.toml", "rust-toolchain.toml", "Cargo.toml"]) {
      copyFileSync(join(REPO, rootName, f), join(rootDir, f));
    }
    for (const c of crates) {
      cpSync(join(REPO, rootName, "crates", c), join(rootDir, "crates", c), { recursive: true, verbatimSymlinks: true });
    }
    const manifest = join(rootDir, "Cargo.toml");
    const text = readFileSync(manifest, "utf8");
    const start = text.indexOf("[workspace]");
    const end = text.indexOf("[workspace.package]");
    const members = crates.map((c) => `  "crates/${c}",`).join("\n");
    writeFileSync(manifest, `${text.slice(0, start)}[workspace]\nresolver = "2"\nmembers = [\n${members}\n]\n\n${text.slice(end)}`);
  }
}

const packages = [
  "axagent-plugin-opc-core",
  "axagent-plugin-opc-accounting",
  "axagent-plugin-opc-sales-growth",
  "axagent-plugin-opc-content-media",
  "axagent-plugin-opc",
  "axagent-plugin-testkit",
  "axagent-plugins",
];
OUT.packages = packages;

// 3) 独立树里跑定点验证。不跑全量，按 AGENTS 分层检查策略逐 crate 定点。
run("cargo", ["metadata", "--no-deps", "--format-version", "1"], sdkRoot, "独立树 cargo metadata");
if (FULL) {
  run("cargo", ["check", "--workspace"], sdkRoot, "全插件树 cargo check --workspace");
}
for (const pkg of packages) {
  run("cargo", ["check", "-p", pkg], sdkRoot, `独立树 cargo check ${pkg}`);
}
for (const pkg of packages.slice(0, -1)) {
  run("cargo", ["clippy", "-p", pkg, "--all-targets", "--", "-D", "warnings"], sdkRoot, `独立树 clippy ${pkg}`);
}
for (const pkg of packages) {
  run("cargo", ["test", "-p", pkg, "--all-targets"], sdkRoot, `独立树 test ${pkg}`);
}

// 4) 独立树 bundle 验收：底册已随 SDK workspace 走，bundle 脚本不得假设主仓布局。
//    accounting 锁独立 worker 全链；security 锁声明式插件（无 worker）链。
run(
  "node",
  ["scripts/build-plugin-bundle.mjs", "--plugin", "opc-accounting", "--check"],
  sdkRoot,
  "独立树 bundle opc-accounting",
);
run(
  "node",
  ["scripts/build-plugin-bundle.mjs", "--plugin", "opc-security", "--check"],
  sdkRoot,
  "独立树 bundle opc-security",
);
run(
  "node",
  ["scripts/build-plugin-bundle.mjs", "--plugin", "opc-software-dev", "--check"],
  sdkRoot,
  "独立树 bundle opc-software-dev",
);

// 5) 独立树产物 → 宿主安装链 e2e：用 SDK 树打包出的 accounting bundle 走
//    PluginManager install → enable → 命令 → disable → 恢复，证明独立树产物
//    不只是「编译得过」，还能被宿主装载并执行。
process.env.AXAGENT_ACCOUNTING_BUNDLE_DIR = join(sdkRoot, "dist", "plugins", "opc-accounting");
run(
  "cargo",
  ["test", "-p", "axagent-plugins", "--test", "e2e_opc_accounting_disable_test", "--all-features"],
  sdkRoot,
  "独立树产物安装链 e2e（accounting）",
);

if (!KEEP) rmSync(TEMP_ROOT, { recursive: true, force: true });
console.log(JSON.stringify(OUT, null, 2));
console.log("SDK_VERIFY_OK");
