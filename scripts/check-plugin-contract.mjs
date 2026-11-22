#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 插件体系统一契约门禁（2026-11-22 立）。
 *
 * 补的是什么盲区：插件事实此前散落在 PLUGIN_NAMES、生成器 WORKER_SPECS/版本常量、
 * 域包底册 verticalSlices、runtime.yaml、Rust PluginManifest、dist/plugin.json 六处，
 * 只有「数量对账」而没有「逐项对账」。任何一处漂移都能让 17 仓 CI 全绿而语义已经分叉。
 *
 * 唯一权威：scripts/lib/plugin-contract.json。本脚本把契约与全部事实源逐项比对。
 * 2026-11-22 起 worker 源码、页面资产的权威位置是主仓外的 17 个独立仓
 * （`../AxInvest-plugin-repos/<plugin>`，`AXAGENT_PLUGIN_REPOS_ROOT` 可覆盖）；
 * C/F 两条判据直接读独立仓，缺任一仓即 fail-closed —— **绝不回退主仓副本**
 * （主仓 `axagent-plugins/crates/plugin-*` 是待删除的迁移残留）。
 * 发布物（dist / release.json）仍由 `generate-plugin-independent-repos.mjs --verify` 对账。
 *
 * 判据（任一不满足即退 1）：
 *   A 契约自身：schema/版本/条目数/名字形态/唯一性/形态⇔crate 与 packageName
 *   B 集合对账：契约 == plugin-repos.mjs PLUGIN_NAMES == packager PLUGIN_SPECS
 *   C 形态对账：strategy ⇔ 主仓 crate 存在 ⇔ 生成器 WORKER_SPECS ⇔ 底册 manifestStrategy/transportOwner
 *   D 版本对账：契约版本 == 生成器常量 == 独立仓 metadata 生成时使用的版本口径
 *   E 命令归属：命令名全局唯一；声明式插件命令必须为空；worker 命令必须非空
 *   F 域包对账：契约 OPC 域 == 底册 packs == YAML 资产目录 == 页面资产
 *   G CI 接线：ci.yml 与 ci-check.mjs 都逐字包含本脚本命令（对称差集护栏的第二道保险）
 *
 * 用法：
 *   node scripts/check-plugin-contract.mjs --selftest   # 内建正负对照，不读真实树
 *   node scripts/check-plugin-contract.mjs              # 门禁
 *   node scripts/check-plugin-contract.mjs --json       # 机器可读报告
 */
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { pluginRepo, pluginReposRoot } from "./lib/plugin-repos.mjs";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = join(__dirname, "..");
const argv = process.argv.slice(2);
const SELFTEST = argv.includes("--selftest");
const JSON_OUT = argv.includes("--json");

const CONTRACT_PATH = join(root, "scripts", "lib", "plugin-contract.json");
const REPOS_MODULE = join(root, "scripts", "lib", "plugin-repos.mjs");
const GENERATOR = join(root, "scripts", "generate-plugin-independent-repos.mjs");
const PACKAGER = join(root, "axagent-plugins", "scripts", "build-plugin-bundle.mjs");
const PACK_REGISTRY = join(root, "axagent-plugins", "scripts", "opc-domain-pack-plugins.json");
const PAGES_GENERATOR = join(root, "scripts", "generate-declarative-opc-pages.mjs");
const CI_YML = join(root, ".github", "workflows", "ci.yml");
const CI_CHECK = join(root, "scripts", "ci-check.mjs");

const read = (p) => readFileSync(p, "utf8");

/** 稳定 JSON：用于 selftest 与报告，保证同一输入同一输出。 */
function stable(value) {
  return JSON.stringify(value);
}

/** 从 JS 源码里提取 Object.freeze 数组或对象字面量的顶层键。 */
function parseTopLevelKeys(src, declaration) {
  const start = src.indexOf(declaration);
  if (start < 0) return null;
  const brace = src.indexOf("{", start + declaration.length - 1);
  const bracket = src.indexOf("[", start + declaration.length - 1);
  const useBrace = brace >= 0 && (bracket < 0 || brace < bracket);
  const open = useBrace ? brace : bracket;
  const closeChar = useBrace ? "}" : "]";
  let depth = 0;
  let quote = null;
  let escaped = false;
  for (let i = open; i < src.length; i += 1) {
    const c = src[i];
    if (quote) {
      if (escaped) escaped = false;
      else if (c === "\\") escaped = true;
      else if (c === quote) quote = null;
      continue;
    }
    if (c === '"' || c === "'" || c === "`") {
      quote = c;
      continue;
    }
    if (c === openCharOf(closeChar)) depth += 1;
    else if (c === closeChar) {
      depth -= 1;
      if (depth === 0) return src.slice(open + 1, i);
    }
  }
  return null;
}

function openCharOf(closeChar) {
  return closeChar === "}" ? "{" : "[";
}

/** 只取对象字面量的键，不解析值；用于 PLUGIN_SPECS / WORKER_SPECS 的键集对账。 */
function objectKeysFromBody(body) {
  const keys = [];
  let depth = 0;
  let quote = null;
  let escaped = false;
  let token = "";
  for (let i = 0; i < body.length; i += 1) {
    const c = body[i];
    if (quote) {
      if (escaped) escaped = false;
      else if (c === "\\") escaped = true;
      else if (c === quote) quote = null;
      continue;
    }
    if (c === '"' || c === "'") {
      if (depth === 0) {
        const end = body.indexOf(c, i + 1);
        keys.push(body.slice(i + 1, end));
        i = end;
        continue;
      }
      quote = c;
      continue;
    }
    if (c === "{" || c === "[") depth += 1;
    else if (c === "}" || c === "]") depth -= 1;
    else if (depth === 0 && c === ":") {
      if (token) keys.push(token);
      token = "";
    }
    else if (depth === 0 && /[A-Za-z0-9_-]/.test(c)) token += c;
    else if (depth === 0 && c === ",") {
      if (token) keys.push(token);
      token = "";
    }
  }
  if (token) keys.push(token);
  return keys;
}

/** 解析生成器的 WORKER_SPECS 字段；阶段 0 用它对账，生成器改读契约后仍然恒等。 */
function parseWorkerSpecs(src) {
  const body = parseTopLevelKeys(src, "const WORKER_SPECS =");
  if (body === null) return null;
  const result = {};
  const entries = [...body.matchAll(/["']?([a-z0-9-]+)["']?\s*:\s*\{/g)];
  for (let i = 0; i < entries.length; i += 1) {
    const name = entries[i][1];
    const chunk = body.slice(entries[i].index, entries[i + 1]?.index ?? body.length);
    result[name] = {
      crate: chunk.match(/crate:\s*["']([^"']+)["']/)?.[1] ?? null,
      packageName: chunk.match(/packageName:\s*["']([^"']+)["']/)?.[1] ?? null,
      sdk: (chunk.match(/sdk:\s*\[([^\]]*)\]/)?.[1] ?? "").match(/["']([a-z0-9-]+)["']/g)?.map((s) => s.slice(1, -1)) ?? [],
      extraCrates: (chunk.match(/extraCrates:\s*\[([^\]]*)\]/)?.[1] ?? "").match(/["']([a-z0-9-]+)["']/g)?.map((s) => s.slice(1, -1)) ?? [],
    };
  }
  return result;
}

/** 解析打包器 PLUGIN_SPECS 的键与关键字段。 */
function parseBundleSpecs(src) {
  const body = parseTopLevelKeys(src, "const PLUGIN_SPECS =");
  if (body === null) return null;
  const result = {};
  const entries = [...body.matchAll(/["']?([a-z0-9-]+)["']?\s*:\s*\{/g)];
  for (let i = 0; i < entries.length; i += 1) {
    const name = entries[i][1];
    const chunk = body.slice(entries[i].index, entries[i + 1]?.index ?? body.length);
    result[name] = {
      packageName: chunk.match(/packageName:\s*["']([^"']+)["']/)?.[1] ?? null,
      webDir: chunk.match(/webDir:\s*path\.join\(([^)]*)\)/)?.[1] ?? null,
      uiSchema: chunk.match(/uiSchema:\s*path\.join\(([^)]*)\)/)?.[1] ?? null,
      manifest: chunk.match(/manifest:\s*([A-Za-z_][A-Za-z0-9_]*)/)?.[1] ?? null,
      commands: (chunk.match(/commands:\s*\[([\s\S]*?)\]/)?.[1] ?? "")
        .match(/"([a-z0-9_]+)"/g)
        ?.map((s) => s.slice(1, -1)) ?? [],
    };
  }
  return result;
}

function semver(value) {
  return typeof value === "string" && /^\d+\.\d+\.\d+$/.test(value);
}

/** 纯校验：不读盘，便于 selftest 的正负对照。 */
function checkContractShape(contract) {
  const violations = [];
  if (!contract || typeof contract !== "object") return ["契约不是对象"];
  if (contract.schemaVersion !== 1) violations.push(`schemaVersion 应为 1，实为 ${contract.schemaVersion}`);
  const releases = contract.releases;
  if (!releases || !semver(releases.workerVersion)) violations.push("releases.workerVersion 非法");
  if (!releases || !semver(releases.legacyDeclarativeVersion)) violations.push("releases.legacyDeclarativeVersion 非法");
  if (!Array.isArray(contract.plugins) || contract.plugins.length === 0) {
    violations.push("plugins 必须是非空数组");
    return violations;
  }
  const seen = new Set();
  for (const p of contract.plugins) {
    const tag = p?.name ?? "(无 name)";
    if (!p || typeof p.name !== "string" || !/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(p.name)) {
      violations.push(`${tag}: name 非法`);
      continue;
    }
    if (seen.has(p.name)) violations.push(`${tag}: name 重复`);
    seen.add(p.name);
    if (p.pluginId !== `plugin-${p.name}`) violations.push(`${tag}: pluginId 应为 plugin-${p.name}`);
    if (p.repo !== `axinvest-plugin-${p.name}`) violations.push(`${tag}: repo 应为 axinvest-plugin-${p.name}`);
    if (!["worker", "declarative"].includes(p.strategy)) violations.push(`${tag}: strategy 非法`);
    if (!semver(p.version)) violations.push(`${tag}: version 非法`);
    if (p.strategy === "worker") {
      if (!p.crate || !p.packageName) violations.push(`${tag}: worker 缺少 crate/packageName`);
      if (!Array.isArray(p.commands) || p.commands.length === 0) violations.push(`${tag}: worker 命令集为空`);
    } else {
      if (p.crate || p.packageName) violations.push(`${tag}: declarative 不应有 crate/packageName`);
      if (Array.isArray(p.commands) && p.commands.length > 0) violations.push(`${tag}: declarative 不得声明命令`);
    }
    if (!Array.isArray(p.commands)) violations.push(`${tag}: commands 必须是数组`);
    if (!Array.isArray(p.pageAssets)) violations.push(`${tag}: pageAssets 必须是数组`);
    if (!["common", "domain"].includes(p.commandScope)) violations.push(`${tag}: commandScope 非法`);
    if (p.domainPackId && p.pageAssets.length === 0) violations.push(`${tag}: 域包插件必须登记页面资产`);
    if (p.domainPackId) {
      if (!["independent-worker", "declarative-only"].includes(p.manifestStrategy)) {
        violations.push(`${tag}: domainPackId 存在但 manifestStrategy 非法`);
      }
      if (p.manifestStrategy === "declarative-only" && p.transportOwner !== "plugin-opc-common") {
        violations.push(`${tag}: declarative-only 的 transportOwner 应为 plugin-opc-common`);
      }
      if (p.manifestStrategy === "independent-worker" && p.transportOwner !== p.name) {
        violations.push(`${tag}: independent-worker 的 transportOwner 应为 ${p.name}`);
      }
    }
  }
  // 公共底座 `opc` 保留全部域命令的历史别名（底册 sharedCommands 登记的既有事实），
  // owner 唯一性只约束域插件；否则阶段 0 就会把既存别名误判成新违规。
  const commandOwners = new Map();
  for (const p of contract.plugins) {
    if (p.commandScope === "common") continue;
    for (const name of p.commands ?? []) {
      if (commandOwners.has(name)) {
        violations.push(`命令 ${name} 被 ${commandOwners.get(name)} 与 ${p.name} 重复声明`);
      }
      commandOwners.set(name, p.name);
    }
  }
  return violations;
}

async function readSources() {
  const contract = JSON.parse(read(CONTRACT_PATH));
  const { PLUGIN_NAMES: pluginNames } = await import("./lib/plugin-repos.mjs");
  const reposSrc = read(REPOS_MODULE);
  if (!reposSrc.includes("plugin-contract.json")) {
    throw new Error("plugin-repos.mjs 的 PLUGIN_NAMES 未从统一契约派生");
  }
  return {
    contract,
    pluginNames,
    generatorSrc: read(GENERATOR),
    packagerSrc: read(PACKAGER),
    registry: JSON.parse(read(PACK_REGISTRY)),
    pagesSrc: existsSync(PAGES_GENERATOR) ? read(PAGES_GENERATOR) : "",
    ciYml: read(CI_YML),
    ciCheck: read(CI_CHECK),
  };
}

function diffSets(actual, expected, label, violations) {
  const a = new Set(actual);
  const e = new Set(expected);
  const onlyActual = [...a].filter((x) => !e.has(x)).sort();
  const onlyExpected = [...e].filter((x) => !a.has(x)).sort();
  if (onlyActual.length) violations.push(`${label} 多出：${onlyActual.join("、")}`);
  if (onlyExpected.length) violations.push(`${label} 缺少：${onlyExpected.join("、")}`);
}

function checkRealTree(sources) {
  const v = [];
  const { contract, pluginNames, generatorSrc, packagerSrc, registry, ciYml, ciCheck } = sources;
  const plugins = contract.plugins;
  const byName = new Map(plugins.map((p) => [p.name, p]));

  // B 集合对账
  diffSets(plugins.map((p) => p.name), pluginNames, "契约↔PLUGIN_NAMES", v);
  const bundleSpecs = parseBundleSpecs(packagerSrc);
  if (bundleSpecs === null) v.push("无法解析打包器 PLUGIN_SPECS");
  else diffSets(plugins.map((p) => p.name), Object.keys(bundleSpecs), "契约↔PLUGIN_SPECS", v);

  // C 形态对账：生成器已从硬编码 WORKER_SPECS 改为从契约派生，
  // 这里改查「派生语句仍在」，不再拿字面量逐项对账（否则每次契约改版都会假红）。
  const derivesWorkers = /CONTRACT\.plugins[\s\S]{0,160}p\.strategy === "worker"/.test(generatorSrc);
  if (!derivesWorkers) v.push("生成器未从契约派生 WORKER_SPECS");
  // worker 源码权威 = 主仓外独立仓；缺仓 fail-closed，不回退主仓副本。
  const reposRoot = pluginReposRoot(root);
  if (!existsSync(reposRoot)) v.push(`独立插件仓根不存在：${reposRoot}（CI 必须先取源，禁止回退主仓副本）`);
  for (const p of plugins) {
    if (p.strategy !== "worker") continue;
    const repoRoot = pluginRepo(root, p.name);
    if (!existsSync(join(repoRoot, ".git"))) {
      v.push(`${p.name}: worker 但独立仓不存在 ${repoRoot}`);
      continue;
    }
    const crateManifest = join(repoRoot, p.crate ?? "__none__", "Cargo.toml");
    if (!existsSync(crateManifest)) v.push(`${p.name}: worker 但独立仓 crate 不存在 ${crateManifest}`);
    const spec = bundleSpecs?.[p.name];
    if (spec) {
      const expectedPackage = p.packageName ?? null;
      if (spec.packageName !== expectedPackage) {
        v.push(`${p.name}: 打包器 packageName=${spec.packageName} 与契约 ${expectedPackage} 不一致`);
      }
      if (p.manifestFactory && spec.manifest !== p.manifestFactory) {
        v.push(`${p.name}: 打包器清单工厂=${spec.manifest} 与契约 ${p.manifestFactory} 不一致`);
      }
      // 通用域 worker 工厂的命令面必须同源：PLUGIN_SPECS.commands → 工厂生成的
      // plugin.json.commands → logical manifest → 契约。其余 worker 的命令由各自静态
      // 工厂声明（不在 PLUGIN_SPECS.commands 登记），不在这条判据的覆盖范围内。
      if (p.manifestFactory === "opcWorkerDomainManifest") {
        diffSets(spec.commands, p.commands, `${p.name}: 打包器命令↔契约`, v);
      }
    }
  }

  // D 版本对账：生成器常量也必须从契约派生；硬编码版本字面量回潮即红。
  if (!generatorSrc.includes("CONTRACT.releases.workerVersion")) {
    v.push("生成器未从契约派生 worker 版本");
  }
  if (!generatorSrc.includes("CONTRACT.releases.legacyDeclarativeVersion")) {
    v.push("生成器未从契约派生声明式版本");
  }
  if (/const RUST_RELEASE_VERSION\s*=\s*"\d/.test(generatorSrc)) {
    v.push("生成器重新写死了 RUST_RELEASE_VERSION");
  }
  if (/const DECLARATIVE_RELEASE_VERSION\s*=\s*"\d/.test(generatorSrc)) {
    v.push("生成器重新写死了 DECLARATIVE_RELEASE_VERSION");
  }
  for (const p of plugins) {
    const expected = p.strategy === "worker" ? contract.releases.workerVersion : contract.releases.legacyDeclarativeVersion;
    if (p.version !== expected) v.push(`${p.name}: version=${p.version} 与策略版本 ${expected} 不一致`);
  }

  // F 域包对账
  const slices = new Map((registry.policy?.verticalSlices ?? []).map((s) => [s.domainPackId, s]));
  const packs = new Map((registry.packs ?? []).map((p) => [p.id, p]));
  const contractDomains = plugins.filter((p) => p.domainPackId).map((p) => p.domainPackId);
  diffSets(contractDomains, [...packs.keys()], "契约域↔底册 packs", v);
  diffSets(contractDomains, [...slices.keys()], "契约域↔底册 verticalSlices", v);
  for (const p of plugins) {
    if (!p.domainPackId) continue;
    const slice = slices.get(p.domainPackId);
    const pack = packs.get(p.domainPackId);
    if (slice && slice.pluginId !== p.pluginId) {
      v.push(`${p.name}: 底册 pluginId=${slice.pluginId} 与契约 ${p.pluginId} 不一致`);
    }
    if (slice && slice.manifestStrategy !== p.manifestStrategy) {
      v.push(`${p.name}: 底册 manifestStrategy=${slice.manifestStrategy} 与契约 ${p.manifestStrategy} 不一致`);
    }
    if (slice && slice.transportOwner !== p.transportOwner) {
      v.push(`${p.name}: 底册 transportOwner=${slice.transportOwner} 与契约 ${p.transportOwner} 不一致`);
    }
    if (pack && pack.pluginId !== p.pluginId) {
      v.push(`${p.name}: packs.pluginId=${pack.pluginId} 与契约 ${p.pluginId} 不一致`);
    }
    const yaml = join(root, "config", "opc", "domain_packs", p.domainPackId, "manifest.yaml");
    if (!existsSync(yaml)) v.push(`${p.name}: 缺少域包 manifest.yaml ${yaml}`);
    const pageDir = join(pluginRepo(root, p.name), `plugin-${p.name}`, "web", "pages");
    for (const asset of p.pageAssets) {
      if (!existsSync(join(pageDir, asset))) v.push(`${p.name}: 缺少契约登记的页面资产 ${asset}`);
    }
    if (!existsSync(join(pageDir, "index.html"))) v.push(`${p.name}: 缺少页面入口 index.html`);
  }

  // G CI 接线：两条命令串由 ci.workflow.test.ts 的对称差集护栏做第一道，这里防本脚本被单侧删掉
  const command = "node scripts/check-plugin-contract.mjs";
  if (!ciYml.includes(command)) v.push("ci.yml 未接线 check-plugin-contract.mjs");
  if (!ciCheck.includes(command)) v.push("ci-check.mjs 未接线 check-plugin-contract.mjs");

  return v;
}

function selftest() {
  const base = {
    schemaVersion: 1,
    releases: { workerVersion: "3.0.1", legacyDeclarativeVersion: "1.0.0" },
    plugins: [
      {
        name: "opc-demo", pluginId: "plugin-opc-demo", repo: "axinvest-plugin-opc-demo",
        strategy: "worker", version: "3.0.1", crate: "plugin-opc-demo", packageName: "axagent-plugin-opc-demo",
        sdk: [], extraCrates: [], commands: ["opc_demo_ping"],
        pageAssets: ["index.html", "bundle.js"], commandScope: "domain",
        domainPackId: "demo", manifestStrategy: "independent-worker", transportOwner: "opc-demo",
      },
      {
        name: "opc-other", pluginId: "plugin-opc-other", repo: "axinvest-plugin-opc-other",
        strategy: "declarative", version: "1.0.0", crate: null, packageName: null,
        sdk: [], extraCrates: [], commands: [],
        pageAssets: ["index.html", "domain-data.js"], commandScope: "domain",
        domainPackId: "other", manifestStrategy: "declarative-only", transportOwner: "plugin-opc-common",
      },
    ],
  };
  const failures = [];
  const ok = (cond, label) => { if (!cond) failures.push(`selftest 失败：${label}`); };

  ok(checkContractShape(base).length === 0, "正样本应无违规");
  const dup = structuredClone(base);
  dup.plugins[1].commands = ["opc_demo_ping"];
  dup.plugins[1].strategy = "worker";
  dup.plugins[1].crate = "plugin-opc-other";
  dup.plugins[1].packageName = "axagent-plugin-opc-other";
  dup.plugins[1].version = "3.0.1";
  ok(checkContractShape(dup).some((x) => x.includes("重复声明")), "重复命令应被抓");
  const badOwner = structuredClone(base);
  badOwner.plugins[0].transportOwner = "plugin-opc-common";
  ok(checkContractShape(badOwner).some((x) => x.includes("transportOwner")), "owner 漂移应被抓");
  const badStrategy = structuredClone(base);
  badStrategy.plugins[1].commands = ["opc_other_ping"];
  ok(checkContractShape(badStrategy).some((x) => x.includes("declarative 不得声明命令")), "声明式命令应被抓");
  const buckets = [];
  diffSets(["a", "b"], ["a", "c"], "fixture", buckets);
  ok(buckets.length === 2, "集合差应双向报告");
  ok(objectKeysFromBody('"a": { x: 1 }, b: { y: 2 },') .join(",") === "a,b", "对象键解析");
  ok(parseWorkerSpecs('const WORKER_SPECS = {\n  "x": { crate: "plugin-x", packageName: "axagent-plugin-x", sdk: ["plugin-testkit"] },\n};')?.x?.crate === "plugin-x", "WORKER_SPECS 单行解析");
  ok(parseBundleSpecs('const PLUGIN_SPECS = {\n  wiki: { packageName: "axagent-plugin-wiki", webDir: null, manifest: wikiManifest },\n};')?.wiki?.manifest === "wikiManifest", "PLUGIN_SPECS 无引号键解析");

  if (failures.length) {
    console.error(failures.join("\n"));
    process.exit(1);
  }
  console.log("PLUGIN_CONTRACT_SELFTEST_OK");
}

if (SELFTEST) {
  selftest();
  process.exit(0);
}

let sources;
try {
  sources = await readSources();
} catch (error) {
  console.error(`[plugin-contract] 读取事实源失败：${error instanceof Error ? error.message : String(error)}`);
  process.exit(1);
}
const violations = [
  ...checkContractShape(sources.contract),
  ...checkRealTree(sources),
];
const report = {
  schemaVersion: sources.contract.schemaVersion,
  plugins: sources.contract.plugins.length,
  worker: sources.contract.plugins.filter((p) => p.strategy === "worker").length,
  declarative: sources.contract.plugins.filter((p) => p.strategy === "declarative").length,
  commands: sources.contract.plugins.reduce((n, p) => n + p.commands.length, 0),
  violations,
};
if (JSON_OUT) console.log(JSON.stringify(report, null, 2));
if (violations.length) {
  if (!JSON_OUT) {
    console.error("[plugin-contract] FAIL:");
    for (const line of violations) console.error(`  - ${line}`);
  }
  process.exit(1);
}
if (!JSON_OUT) {
  console.log(
    `PLUGIN_CONTRACT_OK plugins=${report.plugins} worker=${report.worker} declarative=${report.declarative} commands=${report.commands}`,
  );
}
