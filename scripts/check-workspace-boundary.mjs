#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 跨 workspace 依赖边界门禁（PLAN-repo-split §12.4 / L4）。
 *
 * 判据：物理独立仓终局要求插件 workspace 只通过「已发布 SDK 的版本号」消费 core/quant
 * 等基础能力；同仓阶段用 `{ version, path }` 指向本地 crate。若拷出 `axagent-plugins/`
 * 后 path 仍能解析到主仓目录，说明仍是逻辑拆分而非独立仓。本门把这条形态债显式化：
 *   - 统计每个插件 crate 对 workspace 外根的 `path` 依赖；
 *   - 与 `scripts/workspace-boundary-allowlist.json` 按「crate -> 依赖名」比对；
 *   - 基线只减不增：已有 debt 不计新违规，新 debt / 计数增加即红。
 *
 * 用法：node scripts/check-workspace-boundary.mjs [--selftest] [--update-baseline]
 */
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { discoverWorkspaceRoots } from "./lib/workspace-roots.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const BASELINE_PATH = join(ROOT, "scripts", "workspace-boundary-allowlist.json");
const PLUGIN_WORKSPACE = join(ROOT, "axagent-plugins");
const argv = process.argv.slice(2);
const SELFTEST = argv.includes("--selftest");
const UPDATE = argv.includes("--update-baseline");

const rel = (p) => relative(ROOT, p).split(sep).join("/");

/** 从 Cargo.toml 文本中抽取依赖名与 path（只认 inline table，足够覆盖本仓写法）。 */
export function parsePathDeps(text) {
  const deps = new Map();
  let section = null;
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.replace(/#.*$/, "").trim();
    if (!line) continue;
    const header = /^\[(dependencies|dev-dependencies|build-dependencies)\]$/.exec(line);
    if (header) {
      section = header[1];
      continue;
    }
    if (line.startsWith("[")) {
      section = null;
      continue;
    }
    if (!section) continue;
    const match = /^([\w-]+)\s*=\s*\{([^}]*)\}/.exec(line);
    if (!match) continue;
    const pathMatch = /\bpath\s*=\s*"([^"]+)"/.exec(match[2]);
    if (!pathMatch) continue;
    deps.set(match[1], { path: pathMatch[1], section });
  }
  return deps;
}

/** 逐 crate 统计插件 workspace 对外部 workspace 的 path 依赖。 */
export function scanPluginBoundary(repoRoot = ROOT) {
  const roots = discoverWorkspaceRoots(repoRoot);
  const pluginRoot = join(repoRoot, "axagent-plugins");
  const cratesDir = join(pluginRoot, "crates");
  const findings = {};
  if (!existsSync(cratesDir)) return findings;
  for (const entry of readdirSync(cratesDir, { withFileTypes: true })) {
    if (!entry.isDirectory()) continue;
    const manifest = join(cratesDir, entry.name, "Cargo.toml");
    if (!existsSync(manifest)) continue;
    const deps = parsePathDeps(readFileSync(manifest, "utf8"));
    const outside = [];
    for (const [name, info] of deps) {
      if (!info.path.startsWith("../..")) continue; // 组内 ../x 随插件仓走
      const target = resolve(dirname(manifest), info.path);
      if (target === pluginRoot || target.startsWith(pluginRoot + sep)) continue;
      if (!roots.some((r) => target === r || target.startsWith(r + sep))) continue;
      outside.push(name);
    }
    if (outside.length) findings[entry.name] = outside.sort();
  }
  return findings;
}

function countFindings(findings) {
  return Object.values(findings).reduce((sum, deps) => sum + deps.length, 0);
}

function selftest() {
  const checks = [];
  const t = (name, ok) => checks.push({ name, ok: ok === true });
  const good = parsePathDeps(
    '[dependencies]\naxagent-plugin-proto = { version = "3.0.0", path = "../../../axagent-core/crates/plugin-proto" }\n' +
      'axagent-plugin-opc-core = { version = "3.0.0", path = "../plugin-opc-core" }\n',
  );
  t("解析外部 path 依赖", good.get("axagent-plugin-proto")?.path.includes("axagent-core") === true);
  t("组内 path 依赖也解析", good.get("axagent-plugin-opc-core")?.path === "../plugin-opc-core");
  t("随仓依赖不计越界", good.get("axagent-plugin-opc-core").path.startsWith("../..") === false);
  t("空口径计数为 0", countFindings({}) === 0);
  t("新增 debt 会被计数", countFindings({ "plugin-x": ["axagent-harness"] }) === 1);
  const failed = checks.filter((c) => !c.ok);
  for (const c of checks) console.log(`  ${c.ok ? "✔" : "✖"} ${c.name}`);
  console.log(`selftest：${checks.length - failed.length} passed / ${failed.length} failed`);
  process.exit(failed.length === 0 ? 0 : 1);
}

function main() {
  if (SELFTEST) selftest();
  const actual = scanPluginBoundary();
  if (UPDATE) {
    writeFileSync(BASELINE_PATH, `${JSON.stringify(actual, null, 2)}\n`);
    console.log(`已更新基线：${BASELINE_PATH}（${countFindings(actual)} 条已知 debt）`);
    return;
  }
  const baseline = existsSync(BASELINE_PATH)
    ? JSON.parse(readFileSync(BASELINE_PATH, "utf8"))
    : {};
  const problems = [];
  for (const [crate, deps] of Object.entries(actual)) {
    const allowed = baseline[crate] ?? [];
    for (const dep of deps) {
      if (!allowed.includes(dep)) problems.push(`${crate}: 新增跨 workspace path 依赖 ${dep}（基线未登记）`);
    }
  }
  for (const [crate, deps] of Object.entries(baseline)) {
    if (!actual[crate]) {
      problems.push(`${crate}: 基线 debt 已清零，请运行 --update-baseline 下调`);
      continue;
    }
    for (const dep of deps) {
      if (!actual[crate].includes(dep)) {
        problems.push(`${crate}: 基线 debt ${dep} 已清零，请运行 --update-baseline 下调`);
      }
    }
  }
  if (problems.length) {
    for (const p of problems) console.error(`FAIL: ${p}`);
    process.exit(1);
  }
  console.log(`PASS: 插件 workspace 跨根 path 依赖 ${countFindings(actual)} 条，均已在基线登记（只减不增）`);
}

main();
