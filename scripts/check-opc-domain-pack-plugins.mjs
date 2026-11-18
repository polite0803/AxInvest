#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * P6.2 域包插件边界门禁。
 *
 * 判据来源：`PLAN-p62-domain-pack-plugins.md`。P6.2 的可插单元不是 8+1 能力域，
 * 也不是“一个大 plugin-opc”，而是 config/opc/domain_packs 下的 14 个业务域包。
 * 本脚本只做只读检查：
 *   1. manifest 域包清单 == company-runtime 配置清单 == 域包插件底册；
 *   2. 每个域包 manifest 的 domain 与后端 OPC_CAPABILITY_PACK_DOMAIN 映射一致；
 *   3. 每个域包在底册中恰好绑定一个页面 id / path，且页面 id 不串包；
 *   4. 底册声明的 capabilities 与 manifest 承诺能力逐条相等，插件命令不串包。
 *
 * 用法：node scripts/check-opc-domain-pack-plugins.mjs [--json] [--selftest]
 */

import { readFileSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, "..");
const JSON_OUT = process.argv.includes("--json");
const SELFTEST = process.argv.includes("--selftest");

const REGISTRY_REL = "scripts/opc-domain-pack-plugins.json";
// YAML manifest 权威目录（id / domain / capabilities 的唯一来源）。
const MANIFEST_DIR = "config/opc/domain_packs";
// P6.2 logical manifest 按插件归属分区：公共 0 份 + accounting/sales_growth/content_media 独立 worker 各 1 份 + 11 个声明式独立域各 1 份。
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
  ["plugin-opc-content-media", "axagent-plugins/crates/plugin-opc-content-media/domain_packs"],
]);
const EXPECTED_COUNTS = new Map([
  ["plugin-opc", 0],
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
  ["plugin-opc-content-media", 1],
]);
const CONFIG_REL = "axagent-quant/crates/company-runtime/src/opc/capability_pack_config.rs";
const MAPPING_REL = "src-tauri/src/commands/opc_workflows/mod.rs";

const read = (rel) => readFileSync(join(ROOT, rel), "utf8");
const fail = (msg) => {
  console.error(`FAIL: ${msg}`);
  process.exit(1);
};

export function parseManifestPack(rel, id, text) {
  const ownId = text.match(/^id:\s*([a-z_]+)\s*$/m)?.[1] ?? null;
  const domain = text.match(/^domain:\s*([a-z_]+)\s*$/m)?.[1] ?? null;
  const capabilities = [...text.matchAll(/predicate:\s*\{[^}]*id:\s*"([^"]+)"\}/g)].map((m) => m[1]);
  if (ownId !== id) fail(`${rel}: id=${JSON.stringify(ownId)}，期望 ${id}`);
  if (!domain) fail(`${rel}: 缺 domain 字段`);
  if (capabilities.length === 0) fail(`${rel}: 无 capabilities`);
  return { id, domain, capabilities, rel };
}

export function parseConfigPacks(text) {
  return [...text.matchAll(/^\s*domain_pack_id:\s*"([a-z_]+)"\.to_string\(\)/gm)].map((m) => m[1]);
}

export function parseOpcDomainMapping(text) {
  const block = text.match(/const OPC_CAPABILITY_PACK_DOMAIN[^=]*=\s*&\[([\s\S]*?)\];/)?.[1] ?? "";
  return new Map(
    [...block.matchAll(/\(\s*"([a-z_]+)"\s*,\s*"([a-z_]+)"\s*\)/g)].map((m) => [m[1], m[2]])
  );
}

/** vertical slice 的 owner：accounting 必须独立 worker，security 必须保持共享底座。 */
export function parseVerticalOwner(slice, id) {
  const owner = slice?.transportOwner;
  if (id === "accounting" && owner !== "opc-accounting") {
    throw new Error(`accounting: transportOwner=${owner}，期望 opc-accounting`);
  }
  if (id === "security" && owner !== "plugin-opc-common") {
    throw new Error(`security: transportOwner=${owner}，期望 plugin-opc-common`);
  }
  if (id === "sales_growth" && owner !== "opc-sales-growth") {
    throw new Error(`sales_growth: transportOwner=${owner}，期望 opc-sales-growth`);
  }
  if (id === "content_media" && owner !== "opc-content-media") {
    throw new Error(`content_media: transportOwner=${owner}，期望 opc-content-media`);
  }
  return owner;
}

function sorted(list) {
  return [...list].sort();
}

function equalSet(a, b) {
  const aa = sorted(a);
  const bb = sorted(b);
  return aa.length === bb.length && aa.every((x, i) => x === bb[i]);
}

function selftest() {
  const checks = [];
  const t = (name, ok) => checks.push({ name, ok: ok === true });
  const sample = `# Domain Pack\ndefault:\nid: accounting\ndomain: finance\ncapabilities:\n  - predicate: {kind: id_exact, id: "tool:OpcListInvoices"}\n`;
  const parsed = parseManifestPack("fake.yaml", "accounting", sample);
  t("manifest 解析 id / domain / capability", parsed.domain === "finance" && parsed.capabilities.length === 1);
  t("config 清单解析", equalSet(parseConfigPacks('domain_pack_id: "accounting".to_string();\ndomain_pack_id: "security".to_string();'), ["security", "accounting"]));
  t("OPC 映射解析", parseOpcDomainMapping('const OPC_CAPABILITY_PACK_DOMAIN: &[(&str, &str)] = &[("accounting", "finance")];').get("accounting") === "finance");
  t("集合比较判缺值", !equalSet(["a"], ["a", "b"]));
  t("集合比较判等", equalSet(["a", "b"], ["b", "a"]));
  t("14 独立 + 0 公共 计数口径", EXPECTED_COUNTS.size === 15 && EXPECTED_COUNTS.get("plugin-opc") === 0 && [...EXPECTED_COUNTS.values()].filter((n) => n === 1).length === 14);
  t("独立 owner 合法", parseVerticalOwner({ transportOwner: "opc-accounting" }, "accounting") === "opc-accounting");
  t("错误 owner 被拦", (() => {
    try {
      parseVerticalOwner({ transportOwner: "plugin-opc-common" }, "accounting");
      return false;
    } catch {
      return true;
    }
  })());
  t("security 共享 owner 合法", parseVerticalOwner({ transportOwner: "plugin-opc-common" }, "security") === "plugin-opc-common");
  t("security 错误 owner 被拦", (() => {
    try {
      parseVerticalOwner({ transportOwner: "opc-security" }, "security");
      return false;
    } catch {
      return true;
    }
  })());
  t("content_media 独立 owner 合法", parseVerticalOwner({ transportOwner: "opc-content-media" }, "content_media") === "opc-content-media");
  const failed = checks.filter((c) => !c.ok);
  for (const c of checks) console.log(`  ${c.ok ? "✔" : "✖"} ${c.name}`);
  console.log(`selftest：${checks.length - failed.length} passed / ${failed.length} failed`);
  process.exit(failed.length === 0 ? 0 : 1);
}

function main() {
  if (SELFTEST) selftest();

  const registry = JSON.parse(read(REGISTRY_REL));
  const slices = new Map((registry.policy?.verticalSlices ?? []).map((s) => [s.domainPackId, s]));
  const registryIds = registry.packs.map((p) => p.id);
  const logicalIds = [];
  const countProblems = [];
  for (const [owner, dir] of MANIFEST_DIRS) {
    const files = readdirSync(join(ROOT, dir))
      .filter((f) => f.endsWith(".json"))
      .map((f) => f.replace(/\.json$/, ""))
      .sort();
    const expectedCount = EXPECTED_COUNTS.get(owner);
    if (expectedCount !== undefined && files.length !== expectedCount) {
      countProblems.push(`${dir}: manifest 份数应为 ${expectedCount}，实得 ${files.length}`);
    }
    logicalIds.push(...files);
  }
  const manifestIds = readdirSync(join(ROOT, MANIFEST_DIR), { withFileTypes: true })
    .filter((e) => e.isDirectory())
    .map((e) => e.name)
    .sort();
  const configIds = sorted(parseConfigPacks(read(CONFIG_REL)));
  const mapping = parseOpcDomainMapping(read(MAPPING_REL));

  const problems = [];
  problems.push(...countProblems);
  if (!equalSet(registryIds, logicalIds)) {
    problems.push(`底册与 logical manifest 不一致：底册独有 ${sorted(registryIds).filter((x) => !logicalIds.includes(x))}，manifest 独有 ${logicalIds.filter((x) => !registryIds.includes(x))}`);
  }
  for (const [id, slice] of slices) {
    try {
      parseVerticalOwner(slice, id);
    } catch (error) {
      problems.push(error.message);
    }
  }
  if (!equalSet(registryIds, manifestIds)) {
    problems.push(`底册与 manifest 目录不一致：底册独有 ${sorted(registryIds).filter((x) => !manifestIds.includes(x))}，manifest 独有 ${manifestIds.filter((x) => !registryIds.includes(x))}`);
  }
  if (!equalSet(registryIds, configIds)) {
    problems.push(`底册与 company-runtime 配置不一致：底册独有 ${sorted(registryIds).filter((x) => !configIds.includes(x))}，配置独有 ${configIds.filter((x) => !registryIds.includes(x))}`);
  }

  const pageIds = new Set();
  const seenPages = new Map();
  for (const pack of registry.packs) {
    const manifest = parseManifestPack(`${MANIFEST_DIR}/${pack.id}/manifest.yaml`, pack.id, read(`${MANIFEST_DIR}/${pack.id}/manifest.yaml`));
    if (manifest.domain !== pack.domain) problems.push(`${pack.id}: manifest.domain=${manifest.domain}，底册 domain=${pack.domain}`);
    if (mapping.get(pack.id) !== pack.domain) problems.push(`${pack.id}: 后端映射=${mapping.get(pack.id) ?? "缺失"}，底册 domain=${pack.domain}`);
    if (!pack.page || !pack.page.id || !pack.page.path) {
      problems.push(`${pack.id}: 底册缺 page.id / page.path`);
    } else if (pageIds.has(pack.page.id)) {
      problems.push(`${pack.id}: 页面 id ${pack.page.id} 已属于 ${seenPages.get(pack.page.id)}`);
    } else {
      pageIds.add(pack.page.id);
      seenPages.set(pack.page.id, pack.id);
    }
    if (!equalSet(pack.capabilities, manifest.capabilities)) {
      const missing = manifest.capabilities.filter((c) => !pack.capabilities.includes(c));
      const extra = pack.capabilities.filter((c) => !manifest.capabilities.includes(c));
      problems.push(`${pack.id}: capabilities 不相等，manifest 独有 ${missing.join(",") || "无"}，底册独有 ${extra.join(",") || "无"}`);
    }
  }

  const shared = registry.policy?.sharedCommands ?? [];
  const commandOwners = new Map();
  for (const pack of registry.packs) {
    for (const capability of pack.capabilities) {
      if (!capability.startsWith("tool:Opc")) continue;
      const lower = capability.slice("tool:Opc".length).replace(/([a-z0-9])([A-Z])/g, "$1_$2").toLowerCase();
      const aliases = [lower, ...(pack.commandAliases ?? [])];
      for (const command of aliases) {
        if (shared.includes(command)) continue;
        if (commandOwners.has(command) && commandOwners.get(command) !== pack.id) {
          problems.push(`命令 ${command} 被 ${pack.id} 与 ${commandOwners.get(command)} 同时声明；共享命令必须登记在 policy.sharedCommands`);
        } else {
          commandOwners.set(command, pack.id);
        }
      }
    }
  }

  const result = {
    packCount: registry.packs.length,
    manifestCount: manifestIds.length,
    logicalManifestCount: logicalIds.length,
    configCount: configIds.length,
    commandOwnerCount: commandOwners.size,
    problems,
  };
  if (JSON_OUT) {
    console.log(JSON.stringify(result, null, 2));
  } else if (problems.length) {
    for (const p of problems) console.error(`FAIL: ${p}`);
  } else {
    console.log(`PASS: ${registry.packs.length} 个域包插件边界一致（manifest / config / 映射 / 页面 / capabilities）`);
  }
  process.exit(problems.length === 0 ? 0 : 1);
}

main();
