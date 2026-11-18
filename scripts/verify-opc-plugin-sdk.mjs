#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * OPC 插件 SDK 独立树验收（PLAN-repo-split §12.4 / P6.2 终局形态验证）。
 *
 * 判据：拷出最小 SDK 树（axagent-plugins + axagent-core 根配置 + harness/plugin-proto）
 * 到临时目录后，OPC 六个 crate 仍可 cargo check / clippy / test 全绿。
 * 该脚本不修改主仓源码，只用于连续验证独立仓形态；退出码 0 = 独立树验收通过。
 *
 * 用法：node scripts/verify-opc-plugin-sdk.mjs [--keep]
 */
import { spawnSync } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync, existsSync, copyFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { tmpdir } from "node:os";

const REPO = resolve(import.meta.dirname, "..");
const argv = process.argv.slice(2);
const KEEP = argv.includes("--keep");
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
for (const c of ["harness", "plugin-proto"]) {
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
writeFileSync(
  pluginManifest,
  readFileSync(pluginManifest, "utf8").replace(/  "crates\/plugin-stock",\n/, ""),
);

const packages = [
  "axagent-plugin-opc-core",
  "axagent-plugin-opc-accounting",
  "axagent-plugin-opc-sales-growth",
  "axagent-plugin-opc-content-media",
  "axagent-plugin-opc",
  "axagent-plugins",
];
OUT.packages = packages;

// 3) 独立树里跑定点验证。不跑全量，按 AGENTS 分层检查策略逐 crate 定点。
run("cargo", ["metadata", "--no-deps", "--format-version", "1"], sdkRoot, "独立树 cargo metadata");
for (const pkg of packages) {
  run("cargo", ["check", "-p", pkg], sdkRoot, `独立树 cargo check ${pkg}`);
}
for (const pkg of packages.slice(0, -1)) {
  run("cargo", ["clippy", "-p", pkg, "--all-targets", "--", "-D", "warnings"], sdkRoot, `独立树 clippy ${pkg}`);
}
for (const pkg of packages) {
  run("cargo", ["test", "-p", pkg, "--all-targets"], sdkRoot, `独立树 test ${pkg}`);
}

if (!KEEP) rmSync(TEMP_ROOT, { recursive: true, force: true });
console.log(JSON.stringify(OUT, null, 2));
console.log("SDK_VERIFY_OK");
