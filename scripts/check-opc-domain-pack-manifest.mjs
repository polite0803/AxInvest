#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 域包插件 logical manifest 门禁。
 *
 * P6.2 拆包采用“一域一逻辑清单 + 明确 transport owner”形态：每个域包必须能独立
 * 列出 pluginId、页面、能力、命令与依赖接缝。过渡期域包由公共 worker 承接，批 1 起
 * accounting 由独立 worker `opc-accounting` 承接；owner 必须逐份按 verticalSlices 校验，
 * 不得全局钉死 `plugin-opc-common`。
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
const MANIFEST_DIRS = new Map([
  ["plugin-opc", "axagent-plugins/crates/plugin-opc/domain_packs"],
  ["plugin-opc-accounting", "axagent-plugins/crates/plugin-opc-accounting/domain_packs"],
  ["plugin-opc-ai-research", "axagent-plugins/crates/plugin-opc-ai-research/domain_packs"],
  ["plugin-opc-consulting", "axagent-plugins/crates/plugin-opc-consulting/domain_packs"],
  ["plugin-opc-design", "axagent-plugins/crates/plugin-opc-design/domain_packs"],
  ["plugin-opc-ecommerce", "axagent-plugins/crates/plugin-opc-ecommerce/domain_packs"],
  ["plugin-opc-education", "axagent-plugins/crates/plugin-opc-education/domain_packs"],
  ["plugin-opc-finance-invest", "axagent-plugins/crates/plugin-opc-finance-invest/domain_packs"],
  ["plugin-opc-game-dev", "axagent-plugins/crates/plugin-opc-game-dev/domain_packs"],
  ["plugin-opc-geospatial", "axagent-plugins/crates/plugin-opc-geospatial/domain_packs"],
  ["plugin-opc-project-management", "axagent-plugins/crates/plugin-opc-project-management/domain_packs"],
  ["plugin-opc-security", "axagent-plugins/crates/plugin-opc-security/domain_packs"],
  ["plugin-opc-software-dev", "axagent-plugins/crates/plugin-opc-software-dev/domain_packs"],
  ["plugin-opc-sales-growth", "axagent-plugins/crates/plugin-opc-sales-growth/domain_packs"],
]);
const EXPECTED_COUNTS = new Map([
  ["plugin-opc", 1],
  ["plugin-opc-accounting", 1],
  ["plugin-opc-ai-research", 1],
  ["plugin-opc-consulting", 1],
  ["plugin-opc-design", 1],
  ["plugin-opc-ecommerce", 1],
  ["plugin-opc-education", 1],
  ["plugin-opc-finance-invest", 1],
  ["plugin-opc-game-dev", 1],
  ["plugin-opc-geospatial", 1],
  ["plugin-opc-project-management", 1],
  ["plugin-opc-security", 1],
  ["plugin-opc-software-dev", 1],
  ["plugin-opc-sales-growth", 1],
]);

const read = (rel) => readFileSync(join(ROOT, rel), "utf8");

export function validateManifest(m, expected, options = {}) {
  const problems = [];
  if (m.id !== expected.id) problems.push(`id=${m.id}，期望 ${expected.id}`);
  if (m.pluginId !== expected.pluginId) problems.push(`pluginId=${m.pluginId}，期望 ${expected.pluginId}`);
  const expectedOwner = expected.transportOwner ?? "plugin-opc-common";
  if (m.transportOwner !== expectedOwner) {
    problems.push(`transportOwner=${m.transportOwner}，期望 ${expectedOwner}`);
  }
  if (m.page?.id !== expected.page.id || m.page?.path !== expected.page.path) problems.push("page 与底册不一致");
  const own = [...new Set(m.capabilities ?? [])].sort();
  const want = [...new Set(expected.capabilities)].sort();
  if (own.join("|") !== want.join("|")) problems.push(`capabilities 不一致：manifest=${own.join(",")} / 底册=${want.join(",")}`);
  const shared = options.sharedCommands ?? new Set();
  const sharedCommandsWithPrefix = new Set(
    [...shared].flatMap((name) => [name, name.startsWith("opc_") ? name : `opc_${name}`])
  );
  const hasOpcCommand = expected.capabilities.some(
    (c) => c.startsWith("tool:Opc") && !sharedCommandsWithPrefix.has(commandName(c))
  );
  const expectedCommands = (expected.capabilities ?? [])
    .map((c) => commandName(c))
    .filter((c) => c && !sharedCommandsWithPrefix.has(c));
  const ownCommands = [...new Set(m.commands ?? [])].sort();
  const wantCommands = [...new Set(expectedCommands)].sort();
  if (ownCommands.join("|") !== wantCommands.join("|")) {
    problems.push(`commands 不一致：manifest=${ownCommands.join(",") || "空"} / 期望=${wantCommands.join(",") || "空"}`);
  }
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
  const independent = { ...expected, transportOwner: "opc-accounting" };
  t("独立 owner 合法", validateManifest(
    { ...independent, transportOwner: "opc-accounting", commands: ["opc_get_invoice"], calls: ["opc.store"] },
    independent,
  ).length === 0);
  t("独立 owner 漂移被拦", validateManifest({ ...good, transportOwner: "opc-accounting" }, expected).length > 0);
  t("13 + 1 计数口径", EXPECTED_COUNTS.size === 14 && EXPECTED_COUNTS.get("plugin-opc") === 1 && [...EXPECTED_COUNTS.values()].filter((n) => n === 1).length === 14);
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
  const sharedCommands = new Set(registry.policy?.sharedCommands ?? []);
  const problems = [];
  const manifests = [];
  const commandOwners = new Map();
  for (const [owner, dir] of MANIFEST_DIRS) {
    if (!existsSync(join(ROOT, dir))) {
      fail(`域包 manifest 目录不存在：${dir}`);
    }
    const files = readdirSync(join(ROOT, dir)).filter((f) => f.endsWith(".json")).sort();
    const expectedCount = EXPECTED_COUNTS.get(owner);
    if (expectedCount !== undefined && files.length !== expectedCount) {
      problems.push(`${dir}: manifest 份数应为 ${expectedCount}，实得 ${files.length}`);
    }
    for (const file of files) {
      const rel = `${dir}/${file}`;
      const m = JSON.parse(read(rel));
      const pack = registry.packs.find((p) => p.id === m.id);
      if (!pack) {
        problems.push(`${rel}: id=${m.id} 不在底册`);
        continue;
      }
      const slice = slices.get(pack.id);
      const expected = {
        ...pack,
        pluginId: slice?.pluginId ?? pack.pluginId,
        transportOwner: slice?.transportOwner ?? "plugin-opc-common",
      };
      problems.push(...validateManifest(m, expected, { sharedCommands }).map((p) => `${rel}: ${p}`));
      manifests.push({ id: m.id, owner, pluginId: m.pluginId, commands: m.commands.length, transportOwner: m.transportOwner });
      for (const command of m.commands ?? []) {
        if (commandOwners.has(command) && commandOwners.get(command) !== m.id) {
          problems.push(`命令 ${command} 被 ${m.id} 与 ${commandOwners.get(command)} 同时声明`);
        }
        commandOwners.set(command, m.id);
      }
    }
  }
  const expectedIds = registry.packs.map((p) => p.id).sort();
  const foundIds = manifests.map((m) => m.id).sort();
  if (expectedIds.join("|") !== foundIds.join("|")) {
    problems.push(`域包 logical manifest 清单不一致：期望 ${expectedIds.join(",")}，实得 ${foundIds.join(",")}`);
  }
  for (const [id, slice] of slices) {
    const m = manifests.find((x) => x.id === id);
    if (m && m.pluginId !== slice.pluginId) problems.push(`${id}: pluginId=${m.pluginId}，vertical slice 期望 ${slice.pluginId}`);
    if (m && m.transportOwner !== slice.transportOwner) problems.push(`${id}: transportOwner=${m.transportOwner}，vertical slice 期望 ${slice.transportOwner}`);
  }
  if (JSON_OUT) {
    console.log(JSON.stringify({ manifestCount: manifests.length, manifests, problems }, null, 2));
  } else if (problems.length) {
    for (const p of problems) console.error(`FAIL: ${p}`);
  } else {
    console.log(`PASS: ${manifests.length} 个域包 logical manifest 一致（13 独立 + 1 公共 分区 / 一域一 owner / 页面 / capabilities / 命令 / 接缝）`);
  }
  process.exit(problems.length === 0 ? 0 : 1);
}

main();
