#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 插件源码审计门禁（P6.2 / PLAN §12.2 前置）。
 *
 * 复用宿主 `source_audit.rs` 的规则语义（本仓唯一权威实现是 Rust 侧）做**静态源码审计**：
 * 对 SDK workspace 内每个插件 crate 的 `Cargo.toml` 扫 `git` / `path` 依赖，
 * 并与 `scripts/plugin-source-audit-baseline.json` 按 crate / 依赖名比对（只减不增）。
 * 目标：SDK 独立仓发布前，`path_dependency` 数量必须清零；本门禁保证它不会回涨。
 *
 * 用法：node axagent-plugins/scripts/check-plugin-source-audit.mjs [--selftest] [--update-baseline]
 */
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const BASELINE_PATH = join(ROOT, "scripts", "plugin-source-audit-baseline.json");
const SELFTEST = process.argv.includes("--selftest");
const UPDATE = process.argv.includes("--update-baseline");

/** 复刻 source_audit.rs 的依赖段 / path / git 扫描（静态语义，跨语言镜像）。 */
export function auditManifest(text) {
  const violations = [];
  let inDeps = false;
  for (const [idx, raw] of text.split(/\r?\n/).entries()) {
    const line = raw.trim();
    if (line.startsWith("[")) {
      inDeps = line.includes("dependencies");
      continue;
    }
    if (!inDeps) { continue; }
    const hasGit = /(^|[\s,{])git\s*=/.test(line);
    const hasPath = /(^|[\s,{])path\s*=/.test(line);
    if (hasGit) { violations.push({ rule: "git_dependency", line: idx + 1, detail: line }); }
    if (hasPath) { violations.push({ rule: "path_dependency", line: idx + 1, detail: line }); }
  }
  return violations;
}

/** 扫 SDK workspace 全部插件 crate，返回 crate → {git_dependency[], path_dependency[]}。 */
export function auditPluginCrates(workspaceRoot = ROOT) {
  const cratesDir = join(workspaceRoot, "crates");
  const out = {};
  for (const entry of readdirSync(cratesDir, { withFileTypes: true })) {
    if (!entry.isDirectory()) { continue; }
    const manifest = join(cratesDir, entry.name, "Cargo.toml");
    if (!existsSync(manifest)) { continue; }
    const violations = auditManifest(readFileSync(manifest, "utf8"));
    if (!violations.length) { continue; }
    for (const v of violations) {
      out[entry.name] ??= {};
      out[entry.name][v.rule] ??= [];
      out[entry.name][v.rule].push(v.line);
    }
  }
  return out;
}

function count(violations) {
  return Object.values(violations).reduce((sum, rules) => sum + Object.values(rules).flat().length, 0);
}

function selftest() {
  const checks = [];
  const t = (name, ok) => checks.push({ name, ok: ok === true });
  const clean = auditManifest('[dependencies]\nserde = "1"\n');
  t("无 path/git 依赖不报", clean.length === 0);
  const withPath = auditManifest('[dependencies]\nfoo = { version = "1", path = "../foo" }\n');
  t("path 依赖命中", withPath.some((v) => v.rule === "path_dependency"));
  const withGit = auditManifest('[dependencies]\nfoo = { git = "https://example.com/foo" }\n');
  t("git 依赖命中", withGit.some((v) => v.rule === "git_dependency"));
  const outside = auditManifest('[package]\nrepository = { git = "https://example.com/demo" }\n');
  t("非依赖段不误报", outside.length === 0);
  const multi = auditManifest('[dependencies.foo]\npath = "../foo"\n');
  t("嵌套依赖段也命中", multi.some((v) => v.rule === "path_dependency"));
  const failed = checks.filter((c) => !c.ok);
  for (const c of checks) { console.log(`  ${c.ok ? "✔" : "✖"} ${c.name}`); }
  console.log(`selftest：${checks.length - failed.length} passed / ${failed.length} failed`);
  process.exit(failed.length === 0 ? 0 : 1);
}

function main() {
  if (SELFTEST) { selftest(); }
  const actual = auditPluginCrates(ROOT);
  if (UPDATE) {
    writeFileSync(BASELINE_PATH, `${JSON.stringify(actual, null, 2)}\n`);
    console.log(`已更新基线：${BASELINE_PATH}（${count(actual)} 条已知 debt）`);
    return;
  }
  if (!existsSync(BASELINE_PATH)) {
    console.error(`FAIL: 基线不存在 ${BASELINE_PATH}；先跑 --update-baseline 建立基线`);
    process.exit(1);
  }
  const baseline = JSON.parse(readFileSync(BASELINE_PATH, "utf8"));
  const problems = [];
  for (const [crate, rules] of Object.entries(actual)) {
    const allowed = baseline[crate] ?? {};
    for (const [rule, lines] of Object.entries(rules)) {
      const allowedLines = allowed[rule] ?? [];
      for (const line of lines) {
        if (!allowedLines.includes(line)) {
          problems.push(`${crate}: 新增 ${rule} 依赖（line ${line}，基线未登记）`);
        }
      }
    }
  }
  for (const [crate, rules] of Object.entries(baseline)) {
    if (!actual[crate]) {
      problems.push(`${crate}: 基线 debt 已清零，请跑 --update-baseline 下调`);
      continue;
    }
    for (const [rule, lines] of Object.entries(rules)) {
      const actualLines = actual[crate][rule] ?? [];
      for (const line of lines) {
        if (!actualLines.includes(line)) {
          problems.push(`${crate}: 基线 line ${line} 已清零，请跑 --update-baseline 下调`);
        }
      }
    }
  }
  if (problems.length) {
    for (const p of problems) { console.error(`FAIL: ${p}`); }
    process.exit(1);
  }
  console.log(`PASS: 插件源码审计 debt ${count(actual)} 条，均已在基线登记（只减不增）`);
}

main();
