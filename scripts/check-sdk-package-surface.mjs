#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * SDK 发布物包表面门禁。
 *
 * `cargo package --list` 只证明 Cargo 能组装包，不证明包内包含可编译入口；
 * 若 manifest 指向错误 target，或打包目录混入 `target/`，独立消费者仍会失败。
 * 本门逐个检查三个 SDK crate：
 *   - 必须包含 `Cargo.toml` 与 `src/lib.rs`；
 *   - 不得包含 `target/` 或逃逸到包目录外的路径。
 *
 * 用法：node scripts/check-sdk-package-surface.mjs [--selftest]
 */
import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SELFTEST = process.argv.includes("--selftest");

const SDK_PACKAGES = [
  "axagent-core/crates/plugin-proto/Cargo.toml",
  "axagent-core/crates/harness/Cargo.toml",
  "axagent-plugins/crates/plugin-opc-core/Cargo.toml",
  "axagent-plugins/crates/plugin-testkit/Cargo.toml",
];

/** 归一化 cargo package 输出的相对路径。 */
export function normalizePackagePaths(output) {
  return output
    .split(/\r?\n/)
    .map((line) => line.trim().replace(/^\.\//, ""))
    .filter((line) => line.length > 0 && !line.startsWith("warning:"));
}

/** 返回包表面问题；空数组表示通过。 */
export function validatePackagePaths(manifest, paths, required = ["Cargo.toml", "src/lib.rs"]) {
  const problems = [];
  const set = new Set(paths);
  for (const path of required) {
    if (!set.has(path)) {
      problems.push(`${manifest}: 包内缺少 ${path}`);
    }
  }
  for (const path of paths) {
    if (path.startsWith("/") || path.split("/").includes("..")) {
      problems.push(`${manifest}: 包内出现越界路径 ${path}`);
    }
    if (path === "target" || path.startsWith("target/") || path.includes("/target/")) {
      problems.push(`${manifest}: 包内混入构建目录 ${path}`);
    }
  }
  return problems;
}

function selftest() {
  const checks = [];
  const t = (name, ok) => checks.push({ name, ok: ok === true });

  t(
    "合法包通过",
    validatePackagePaths("demo/Cargo.toml", ["Cargo.toml", "src/lib.rs", ".cargo_vcs_info.json"]).length === 0,
  );
  t(
    "缺 lib.rs 被拦",
    validatePackagePaths("demo/Cargo.toml", ["Cargo.toml"]).some((p) => p.includes("缺少 src/lib.rs")),
  );
  t(
    "target 被拦",
    validatePackagePaths("demo/Cargo.toml", ["Cargo.toml", "src/lib.rs", "target/debug/demo"]).some((p) =>
      p.includes("构建目录")
    ),
  );
  t(
    "越界路径被拦",
    validatePackagePaths("demo/Cargo.toml", ["Cargo.toml", "src/lib.rs", "../outside"]).some((p) =>
      p.includes("越界路径")
    ),
  );
  t(
    "输出归一化去噪",
    normalizePackagePaths("warning: demo\nCargo.toml\n./src/lib.rs\n").join(",") === "Cargo.toml,src/lib.rs",
  );

  const failed = checks.filter((check) => !check.ok);
  for (const check of checks) {
    console.log(`  ${check.ok ? "✔" : "✖"} ${check.name}`);
  }
  console.log(`selftest：${checks.length - failed.length} passed / ${failed.length} failed`);
  process.exit(failed.length === 0 ? 0 : 1);
}

function packagePaths(manifest) {
  const manifestPath = resolve(ROOT, manifest);
  const result = spawnSync(
    "cargo",
    ["package", "--list", "--allow-dirty", "--manifest-path", manifestPath],
    { cwd: ROOT, encoding: "utf8" },
  );
  if (result.error) {
    return { error: `cargo 启动失败：${result.error.message}` };
  }
  if (result.status !== 0) {
    const detail = (result.stderr || result.stdout || "").trim();
    return { error: `cargo package --list 失败（exit ${result.status}）：${detail}` };
  }
  return { paths: normalizePackagePaths(result.stdout) };
}

function main() {
  if (SELFTEST) {
    selftest();
  }

  const problems = [];
  for (const manifest of SDK_PACKAGES) {
    const result = packagePaths(manifest);
    if (result.error) {
      problems.push(`${manifest}: ${result.error}`);
      continue;
    }
    problems.push(...validatePackagePaths(manifest, result.paths));
  }

  if (problems.length) {
    for (const problem of problems) {
      console.error(`FAIL: ${problem}`);
    }
    process.exit(1);
  }
  console.log(`PASS: SDK 包表面完整（${SDK_PACKAGES.length} 个 crate，均含 Cargo.toml / src/lib.rs 且无 target）`);
}

main();
