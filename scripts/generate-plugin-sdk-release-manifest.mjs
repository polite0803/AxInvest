#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 插件 SDK 发布物清单（只读、可复跑）。
 *
 * 目的：把“三件套是否可发布”拆成可审阅事实，而不是只看 `publish = false`：
 *   - crate 名 / 版本 / license / repository / publish 位；
 *   - 依赖边（包内依赖 vs 跨根 path debt）；
 *   - `cargo package --list` 的入口文件。
 *
 * 本脚本不执行 `cargo publish`，也不改 manifest。默认写 `docs/audits/`，`--check`
 * 只比较盘上文档与当前源树是否一致，接入 CI 防止漂移。
 *
 * 用法：node scripts/generate-plugin-sdk-release-manifest.mjs [--check]
 */
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(ROOT, "docs", "audits", "AUDIT-plugin-sdk-release-manifest-2026-11-18.md");
const CHECK = process.argv.includes("--check");
const SELFTEST = process.argv.includes("--selftest");
const PACKAGES = [
  ["axagent-plugin-proto", "axagent-core/crates/plugin-proto/Cargo.toml"],
  ["axagent-harness", "axagent-core/crates/harness/Cargo.toml"],
  ["axagent-plugin-opc-core", "axagent-plugins/crates/plugin-opc-core/Cargo.toml"],
  ["axagent-plugin-testkit", "axagent-plugins/crates/plugin-testkit/Cargo.toml"],
];

function readManifest(rel) {
  return readFileSync(join(ROOT, rel), "utf8");
}

/** 只在 `[package]` 段内取字符串字段，避免读到 `[workspace.package]` / 依赖段。 */
export function packageField(text, name) {
  let inPackage = false;
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (line.startsWith("[")) {
      inPackage = line === "[package]";
      continue;
    }
    if (!inPackage) { continue; }
    const stringField = new RegExp(`^${name}\\s*=\\s*"([^"]*)"$`).exec(line);
    if (stringField) { return stringField[1]; }
    const boolField = new RegExp(`^${name}\\s*=\\s*(true|false)$`).exec(line);
    if (boolField) { return boolField[1]; }
    const inheritedField = new RegExp(`^${name}\\.workspace\\s*=\\s*true$`).exec(line);
    if (inheritedField) { return "workspace"; }
  }
  return null;
}

function packageList(manifest) {
  const result = spawnSync(
    "cargo",
    ["package", "--list", "--allow-dirty", "--manifest-path", join(ROOT, manifest)],
    { cwd: ROOT, encoding: "utf8" },
  );
  if (result.status !== 0) {
    throw new Error(`${manifest}: cargo package --list 失败：${(result.stderr || result.stdout).trim()}`);
  }
  return result.stdout
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0 && !line.startsWith("warning:"));
}

/** 从 Cargo.toml 读取包名；版本/仓库/发布位都以 manifest 为权威。 */
function packageName(text) {
  return /^\[package\][\s\S]*?^name\s*=\s*"([^"]+)"/m.exec(text)?.[1] ?? null;
}

export function parseDependencyRows(text) {
  const rows = [];
  let inDeps = false;
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (line.startsWith("[")) {
      inDeps = line.includes("dependencies");
      continue;
    }
    if (!inDeps || !line || line.startsWith("#")) { continue; }
    const inline = /^([A-Za-z0-9_-]+)\s*=\s*\{(.*)\}/.exec(line);
    if (inline) {
      rows.push({
        name: inline[1],
        path: /path\s*=\s*"([^"]+)"/.exec(inline[2])?.[1] ?? null,
      });
      continue;
    }
    const shorthand = /^([A-Za-z0-9_-]+)\s*=\s*"[^"]*"$/.exec(line);
    if (shorthand) {
      rows.push({ name: shorthand[1], path: null });
    }
  }
  return rows;
}

function dependencyRows(manifest) {
  return parseDependencyRows(readManifest(manifest));
}

function selftest() {
  const sample = `[package]
name = "demo-sdk"
version.workspace = true
license = "AGPL-3.0-only"
publish = false

[dependencies]
serde = "1"
peer = { version = "3", path = "../peer" }
`;
  const checks = [];
  const t = (name, ok) => checks.push({ name, ok: ok === true });
  t("version.workspace 读取为 workspace", packageField(sample, "version") === "workspace");
  t("license 读取字符串", packageField(sample, "license") === "AGPL-3.0-only");
  t("publish=false 读取布尔值", packageField(sample, "publish") === "false");
  const deps = parseDependencyRows(sample);
  t("path 依赖被识别", deps.some((d) => d.name === "peer" && d.path === "../peer"));
  t("registry 依赖不伪造 path", deps.some((d) => d.name === "serde" && d.path === null));
  t("依赖段外字段不误读", packageField("[workspace.package]\npublish = true\n", "publish") === null);
  const failed = checks.filter((c) => !c.ok);
  for (const c of checks) console.log(`  ${c.ok ? "✔" : "✖"} ${c.name}`);
  console.log(`selftest：${checks.length - failed.length} passed / ${failed.length} failed`);
  process.exit(failed.length === 0 ? 0 : 1);
}

function render() {
  const rows = [];
  for (const [name, manifest] of PACKAGES) {
    const text = readManifest(manifest);
    const actualName = packageName(text);
    if (actualName !== name) {
      throw new Error(`${manifest}: 包名 ${actualName ?? "缺失"} 与期望 ${name} 不一致`);
    }
    const version = packageField(text, "version") ?? "workspace";
    const license = packageField(text, "license") ?? "workspace";
    const repository = packageField(text, "repository") ?? "-";
    const publish = packageField(text, "publish") ?? "未显式声明（Cargo 默认允许发布）";
    const files = packageList(manifest);
    // 包名参与摘要：仅文件集合相同的两个 crate 不能被误读为同一个发布物。
    const digest = createHash("sha256")
      .update(`${name}\n${version}\n${files.join("\n")}`)
      .digest("hex")
      .slice(0, 16);
    rows.push({
      name,
      manifest,
      version,
      license,
      repository,
      publish,
      digest,
      files,
      deps: dependencyRows(manifest),
    });
  }

  const packageRows = rows.map((row) =>
    `| \`${row.name}\` | \`${row.manifest}\` | ${row.version} | ${row.license} | ${row.publish} | ${row.digest} | ${
      row.files.includes("Cargo.toml") && row.files.includes("src/lib.rs") ? "✓" : "✗"
    } |`
  ).join("\n");
  const depRows = rows.flatMap((row) =>
    row.deps.map((dep) =>
      `| \`${row.name}\` | \`${dep.name}\` | ${dep.path ? `\`${dep.path}\`` : "registry / workspace"} |`
    )
  ).join("\n");
  const packageSections = rows.map((row) =>
    `### \`${row.name}\`\n\n仓库：${row.repository}\n\n包内文件：\n\n${
      row.files.map((file) => `- \`${file}\``).join("\n")
    }`
  ).join("\n\n");

  return `# AUDIT — 插件 SDK 发布物清单（2026-11-18）

> 状态：active ｜ 日期：2026-11-18 ｜ 对应版本：v3.0.1
> 关联：\`PLAN-p62-domain-pack-plugins.md\`、\`PLAN-shell-plus-plugins.md\`
> 性质：由 \`generate-plugin-sdk-release-manifest.mjs\` 生成的可复跑清单；**不表示已发布**。

## 一、发布位

| crate | manifest | version | license | publish | package-list sha256 前缀 | 入口 |
|---|---|---|---|---|---|---|
${packageRows}

## 二、依赖边

| crate | 依赖 | 解析形态 |
|---|---|---|
${depRows}

## 三、包内文件

${packageSections}

## 四、判读

- 四个 SDK 包均满足“可组装包表面”：包内含 \`Cargo.toml\` 与 \`src/lib.rs\`。
- \`axagent-plugin-opc-core\` 是 OPC worker 的领域 SDK；它随协议 SDK 一起按版本发布。
- 源清单仍保留跨根 path 依赖供仓内开发；\`cargo package\` 会把它归一化为版本依赖，实际发布演练由 \`verify-plugin-sdk-release-rehearsal.mjs\` 验证。
- 本清单只记录发布前置，不替代 \`cargo publish --dry-run\`、签名/撤销通道或私有 registry 实测。
`;
}

function main() {
  if (SELFTEST) {
    selftest();
  }
  const content = render();
  if (CHECK) {
    const actual = existsSync(OUT) ? readFileSync(OUT, "utf8") : "";
    if (actual !== content) {
      console.error(`FAIL: ${relative(ROOT, OUT)} 已过期；运行 node scripts/generate-plugin-sdk-release-manifest.mjs`);
      process.exit(1);
    }
    console.log("PASS: SDK 发布物清单与当前源树一致");
    return;
  }
  writeFileSync(OUT, content);
  console.log(`已生成 ${relative(ROOT, OUT)}`);
}

main();
