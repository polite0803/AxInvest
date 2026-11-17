#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 域包插件 logical manifest 门禁。
 *
 * P6.2 拆包采用“一域一逻辑清单 + 公共 transport worker”过渡形态：每个域包必须能独立
 * 列出 pluginId、页面、能力、命令与依赖接缝；命令实际由公共 worker 承接，但不得把
 * 公共 OPC 全量命令复制进单个域包清单。
 *
 * 用法：node scripts/check-opc-domain-pack-manifest.mjs [--json] [--selftest]
 */

import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, "..");
const JSON_OUT = process.argv.includes("--json");
const SELFTEST = process.argv.includes("--selftest");
const REGISTRY_REL = "scripts/opc-domain-pack-plugins.json";
const MANIFEST_DIR = "axagent-plugins/crates/plugin-opc/domain_packs";

const read = (rel) => readFileSync(join(ROOT, rel), "utf8");

export function validateManifest(m, expected) {
  const problems = [];
  if (m.id !== expected.id) problems.push(`id=${m.id}，期望 ${expected.id}`);
  if (m.pluginId !== expected.pluginId) problems.push(`pluginId=${m.pluginId}，期望 ${expected.pluginId}`);
  if (m.transportOwner !== "plugin-opc-common") problems.push(`transportOwner=${m.transportOwner}，必须是 plugin-opc-common`);
  if (m.page?.id !== expected.page.id || m.page?.path !== expected.page.path) problems.push("page 与底册不一致");
  const own = [...new Set(m.capabilities ?? [])].sort();
  const want = [...new Set(expected.capabilities)].sort();
  if (own.join("|") !== want.join("|")) problems.push(`capabilities 不一致：manifest=${own.join(",")} / 底册=${want.join(",")}`);
  const hasOpcCommand = expected.capabilities.some((c) => c.startsWith("tool:Opc"));
  if (hasOpcCommand && (m.commands ?? []).length === 0) problems.push("commands 为空");
  if (!hasOpcCommand && (m.commands ?? []).length !== 0) problems.push("无 tool:Opc 能力时 commands 必须为空");
  if (!m.calls?.includes("opc.store")) problems.push("calls 缺 opc.store");
  return problems;
}

export function commandName(capability) {
  if (!capability.startsWith("tool:Opc")) return null;
  return `opc_${capability.slice("tool:Opc".length).replace(/([a-z0-9])([A-Z])/g, "$1_$2").toLowerCase()}`;
}

function selftest() {
  const checks = [];
  const t = (name, ok) => checks.push({ name, ok: ok === true });
  const expected = { id: "a", pluginId: "plugin-opc-a", page: { id: "p", path: "/a" }, capabilities: ["tool:OpcGetInvoice"] };
  const good = { ...expected, transportOwner: "plugin-opc-common", commands: ["opc_get_invoice"], calls: ["opc.store"] };
  t("合法 manifest 通过", validateManifest(good, expected).length === 0);
  t("pluginId 漂移被拦", validateManifest({ ...good, pluginId: "plugin-opc-b" }, expected).length > 0);
  t("transportOwner 漂移被拦", validateManifest({ ...good, transportOwner: "plugin-opc-a" }, expected).length > 0);
  t("commandName 小写转 snake", commandName("tool:OpcGetFinancialReport") === "opc_get_financial_report");
  t("非 OPC 能力无命令名", commandName("tool:WebSearch") === null);
  const failed = checks.filter((c) => !c.ok);
  for (const c of checks) console.log(`  ${c.ok ? "✔" : "✖"} ${c.name}`);
  console.log(`selftest：${checks.length - failed.length} passed / ${failed.length} failed`);
  process.exit(failed.length === 0 ? 0 : 1);
}

function main() {
  if (SELFTEST) selftest();
  const registry = JSON.parse(read(REGISTRY_REL));
  const slices = new Map((registry.policy?.verticalSlices ?? []).map((s) => [s.domainPackId, s]));
  const problems = [];
  const manifests = [];
  if (!existsSync(join(ROOT, MANIFEST_DIR))) {
    fail(`域包 manifest 目录不存在：${MANIFEST_DIR}`);
  }
  const files = readdirSync(join(ROOT, MANIFEST_DIR)).filter((f) => f.endsWith(".json")).sort();
  for (const file of files) {
    const m = JSON.parse(read(`${MANIFEST_DIR}/${file}`));
    const pack = registry.packs.find((p) => p.id === m.id);
    if (!pack) {
      problems.push(`${file}: id=${m.id} 不在底册`);
      continue;
    }
    const expected = { ...pack, pluginId: slices.get(pack.id)?.pluginId ?? pack.pluginId };
    problems.push(...validateManifest(m, expected).map((p) => `${file}: ${p}`));
    manifests.push({ id: m.id, pluginId: m.pluginId, commands: m.commands.length, transportOwner: m.transportOwner });
  }
  const expectedIds = registry.packs.map((p) => p.id).sort();
  const foundIds = manifests.map((m) => m.id).sort();
  if (expectedIds.join("|") !== foundIds.join("|")) {
    problems.push(`域包 logical manifest 清单不一致：期望 ${expectedIds.join(",")}，实得 ${foundIds.join(",")}`);
  }
  for (const [id, slice] of slices) {
    const m = manifests.find((x) => x.id === id);
    if (m && m.pluginId !== slice.pluginId) problems.push(`${id}: pluginId=${m.pluginId}，vertical slice 期望 ${slice.pluginId}`);
  }
  if (JSON_OUT) {
    console.log(JSON.stringify({ manifestCount: manifests.length, manifests, problems }, null, 2));
  } else if (problems.length) {
    for (const p of problems) console.error(`FAIL: ${p}`);
  } else {
    console.log(`PASS: ${manifests.length} 个域包 logical manifest 一致（一域一 pluginId / 页面 / capabilities / 命令 / 接缝）`);
  }
  process.exit(problems.length === 0 ? 0 : 1);
}

main();
