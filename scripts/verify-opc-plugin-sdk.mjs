#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * OPC 插件 SDK 独立树验收（PLAN-repo-split §12.4 / P6.2 终局形态验证）。
 *
 * 判据：主仓拆仓后，SDK 独立仓（plugin-opc-core / plugin-testkit）仍可从仓内源码
 * 完成 cargo check / clippy / test，并从外置插件仓构建真实 bundle。
 * 该脚本不修改主仓源码，只用于连续验证独立仓形态；退出码 0 = 独立树验收通过。
 *
 * 用法：node scripts/verify-opc-plugin-sdk.mjs [--keep] [--full]
 *  --full：拷出全部插件 workspace（含 quant/ai/infra/kit 最小依赖）并跑 cargo check --workspace。
 */
import { spawnSync } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync, copyFileSync, symlinkSync } from "node:fs";
import { join, resolve } from "node:path";
import { tmpdir } from "node:os";
import { pluginRepo } from "./lib/plugin-repos.mjs";

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

// 1) SDK crate 以主仓保留的 contract/tooling 目录为准，插件 worker 以独立仓为准。
const sdkRoot = join(TEMP_ROOT, "axagent-plugins");
const coreRoot = join(TEMP_ROOT, "axagent-core");
mkdirSync(sdkRoot, { recursive: true });
mkdirSync(coreRoot, { recursive: true });
cpSync(join(REPO, "axagent-plugins", "Cargo.toml"), join(sdkRoot, "Cargo.toml"));
for (const dir of ["plugin-opc-core", "plugin-testkit", "plugins", "schema-gen"]) {
  cpSync(
    join(REPO, "axagent-plugins", "crates", dir),
    join(sdkRoot, "crates", dir),
    { recursive: true, verbatimSymlinks: true },
  );
}
for (const plugin of ["opc", "opc-accounting", "opc-sales-growth", "opc-content-media"]) {
  const repo = pluginRepo(REPO, plugin);
  cpSync(join(repo, `plugin-${plugin}`), join(sdkRoot, "crates", `plugin-${plugin}`), {
    recursive: true,
    verbatimSymlinks: true,
  });
}
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
writeFileSync(
  pluginManifest,
  readFileSync(pluginManifest, "utf8").replace(
    /\[workspace\][\s\S]*?members\s*=\s*\[[\s\S]*?\]/,
    "[workspace]\nresolver = \"2\"\nmembers = [\n  \"crates/plugin-opc\",\n  \"crates/plugin-opc-accounting\",\n  \"crates/plugin-opc-content-media\",\n  \"crates/plugin-opc-core\",\n  \"crates/plugin-opc-sales-growth\",\n  \"crates/plugin-testkit\",\n  \"crates/plugins\",\n  \"crates/schema-gen\",\n]",
  ),
);
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

// 全量独立树会跑到 stock 卸载隔离 e2e；该用例明确要求“缺真 bundle 即失败”，
// 不为独立树单独降级成 SKIP，否则这条隔离证据会被悄悄掏空。
if (FULL) {
  // stock 页面自检依赖 jsdom，且 build-plugin-bundle.mjs 会把 process.env 原样传给
  // 页面校验子进程（ESM import 不认 NODE_PATH）。链接主仓依赖根后，临时树与独立仓
  // 生成器 npm install 后的解析行为一致，且不改动主仓锁文件。
  symlinkSync(join(REPO, "node_modules"), join(sdkRoot, "node_modules"), "dir");
  for (const plugin of ["stock", "opc-security"]) {
    const r = spawnSync(
      "node",
      ["scripts/build-plugin-bundle.mjs", "--plugin", plugin, "--check"],
      { cwd: sdkRoot, stdio: "inherit", windowsHide: true },
    );
    OUT.steps.push({ label: `独立树 staging ${plugin} bundle`, command: "node", exitCode: r.status });
    if (r.status !== 0) fail(`独立树 staging ${plugin} bundle（exit ${r.status}）`);
  }
  process.env.AXAGENT_STOCK_BUNDLE_DIR = join(sdkRoot, "dist", "plugins", "stock");
  process.env.AXAGENT_SECURITY_BUNDLE_DIR = join(sdkRoot, "dist", "plugins", "opc-security");
}

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
  const args = ["test", "-p", pkg, "--all-targets"];
  // 非 FULL 独立树按设计剔除了 plugin-stock：stock 卸载隔离用例依赖两份真 bundle，
  // 不属于这条 OPC 最小树验收。这里显式跳过它，避免“非全量树必然红”；FULL 树则
  // 在上方 staging stock/security 真产物，并用原样 --all-targets 覆盖该用例。
  if (!FULL && pkg === "axagent-plugins") {
    args.push("--", "--skip", "uninstalling_stock_leaves_security_plugin_available");
  }
  // 与 scripts/cargo-all-workspaces.mjs 的 staging 口径一致：stock worker 同时供
  // bundle 安装与工具帧 e2e 使用；只给 STOCK_BUNDLE_DIR 会让另一个用例静默 SKIP。
  if (FULL && pkg === "axagent-plugins") {
    process.env.AXAGENT_STOCK_WORKER_BIN = join(
      process.env.AXAGENT_STOCK_BUNDLE_DIR,
      "bin",
      process.platform === "win32" ? "axagent-plugin-stock.exe" : "axagent-plugin-stock",
    );
  }
  run("cargo", args, sdkRoot, `独立树 test ${pkg}`);
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
