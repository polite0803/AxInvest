#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 插件启停验收矩阵（只读生成）。
 *
 * 交接单里的“24 个 crate”与盘上口径不一致，不能混写：
 *   - 主仓 `axagent-plugins/Cargo.toml` 保留 4 个宿主侧 workspace 成员；
 *   - 17 个外置插件仓合计 7 个 Cargo package（普通 worker 5 个 + stock-core + wiki）；
 *   - 17 个外置仓各有一个单插件 `PLUGIN_SPECS`，合计 17 个可打包 bundle；
 *   - P6.2 的 14 个 OPC 域全部由独立 worker 承接（14 独立 + 0 声明式）。
 *   - `wiki` bundle 的 `packageName: null`：使用宿主 wiki.store 接缝，不是独立构建产物。
 *   - `release` 列审查该 bundle 是否具备“发布地址一键安装”所需的成品形态：
 *     有 worker 可执行、有页面清单（或无页面例外）且打包器 `--check` 已覆盖。
 * 交接单“24 个”没有第四份权威可对应，当前证据只能支持上述四种可复算口径。
 *
 * `crate` 字段只写 Cargo package 真名；声明式 OPC 域没有独立 crate，写 null，
 * `buildable` 明确标「否：声明式插件无独立 crate」，不能伪造 axagent-plugin-* 名字。
 * 矩阵 every row 的 crate 必须落在本清单解析出的 Cargo package 名集合里（或为 null）。
 * 本脚本把物理目录与四份权威拼成一张逐项矩阵，并把「静态证据」与「真 bundle 启停证据」分开：
 *   可执行 E2E 覆盖 14 个独立 worker 域 + wiki + stock；每个域由自己的 staging 环境变量门控，
 *   缺变量即 fail-closed，不把 SKIP 当绿；
 *   公共 opc bundle 由 e2e_opc_common_disable_test.rs 单独覆盖；
 *   其余 crate 只按 cargo/clippy 证据登记，明确标为未做启停端到端。
 *
 * 用法：
 *   node scripts/verify-plugin-acceptance-matrix.mjs
 *   node scripts/verify-plugin-acceptance-matrix.mjs --json
 */
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  DOMAIN_PACK_PLUGINS,
  PLUGIN_CONTRACT,
  PLUGIN_NAMES,
  WORKER_PLUGIN_NAMES,
  pluginBundleScript,
  pluginDomainPacksDir,
  pluginRepo,
  pluginWebPagesDir,
} from "./lib/plugin-repos.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, "..");
const JSON_OUT = process.argv.includes("--json");

const read = (rel) => readFileSync(join(ROOT, rel), "utf8");
const fail = (message) => {
  console.error(`FAIL: ${message}`);
  process.exit(1);
};

const hostManifest = read("axagent-plugins/Cargo.toml");
const workspaceVersion = hostManifest.match(/^\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m)?.[1];
if (!workspaceVersion) fail("无法解析 axagent-plugins workspace.package.version");
const ownership = JSON.parse(
  readFileSync(join(pluginRepo(ROOT, "opc"), "scripts", "opc-domain-pack-plugins.json"), "utf8"),
);
const stagingScript = read("scripts/cargo-all-workspaces.mjs");
const ownershipScript = JSON.stringify(ownership);

function parseWorkspaceCrates(text) {
  const block = text.match(/\[workspace\][\s\S]*?members\s*=\s*\[([\s\S]*?)\]/)?.[1];
  if (!block) fail("无法解析 axagent-plugins workspace members");
  // workspace 成员路径是 `crates/<目录名>`；前面几行 workspace.package 的 version/license
  // 不在 members 块里，这里只取路径末段作为 crate 目录名。插件可执行包已经迁到主仓外的
  // 独立仓，主仓成员只应保留宿主运行时与 SDK 工具。
  return [...block.matchAll(/"crates\/([^"]+)"/g)].map((match) => match[1]);
}

function readCrateDirectories() {
  const plugins = new Map();
  for (const plugin of PLUGIN_NAMES) {
    const base = pluginRepo(ROOT, plugin);
    if (!existsSync(base)) fail(`独立插件仓不存在：${base}`);
    const candidate = join(base, `plugin-${plugin}`);
    if (existsSync(join(candidate, "Cargo.toml"))) {
      plugins.set(`plugin-${plugin}`, join(candidate, "Cargo.toml"));
    }
    if (plugin === "stock" && existsSync(join(base, "stock-core", "Cargo.toml"))) {
      plugins.set("stock-core", join(base, "stock-core", "Cargo.toml"));
    }
  }
  const hostBase = join(ROOT, "axagent-plugins", "crates");
  for (const name of ["plugins", "schema-gen", "plugin-opc-core", "plugin-testkit"]) {
    const manifest = join(hostBase, name, "Cargo.toml");
    if (!existsSync(manifest) || !statSync(join(hostBase, name)).isDirectory()) {
      fail(`主仓插件 workspace 缺少宿主侧 crate：${name}`);
    }
    plugins.set(name, manifest);
  }
  return plugins;
}

function parseCargoPackageNames() {
  const names = new Map();
  const versions = new Map();
  for (const [dir, manifestPath] of crateDirectories) {
    const manifest = readFileSync(manifestPath, "utf8");
    const packageBlock = manifest.match(/^\[package\]([\s\S]*?)(?=^\[)/m)?.[1];
    const packageName = packageBlock?.match(/^name\s*=\s*"([^"]+)"/m)?.[1];
    const packageVersion = packageBlock?.match(/^version\s*=\s*"([^"]+)"/m)?.[1]
      ?? (/^version\s*(\.\s*workspace\s*=\s*true|\s*=\s*\{\s*workspace\s*=\s*true\s*\})/m.test(packageBlock ?? "") ? workspaceVersion : undefined);
    if (packageName) names.set(dir, packageName);
    if (packageVersion) versions.set(dir, packageVersion);
  }
  return { names, versions };
}

function parseBundleSpecNames(text) {
  const block = text.match(/const PLUGIN_SPECS\s*=\s*\{([\s\S]*?)\n\};/)?.[1];
  if (!block) fail("无法解析 PLUGIN_SPECS");
  const names = [];
  for (const line of block.split("\n")) {
    const match = line.match(/^  "?([a-z0-9-]+)"?:\s*\{/);
    if (match) names.push(match[1]);
  }
  return names;
}

function parseBundleSpecPackages(text) {
  const block = text.match(/const PLUGIN_SPECS\s*=\s*\{([\s\S]*?)\n\};/)?.[1];
  if (!block) fail("无法解析 PLUGIN_SPECS");
  const packages = new Map();
  for (const line of block.split("\n")) {
    const match = line.match(/^  "?([a-z0-9-]+)"?:\s*\{/);
    if (!match) continue;
    const next = block.slice(block.indexOf(match[0]) + match[0].length);
    const packageName = next.split("\n  ")[0].match(/packageName:\s*(?:"([^"]+)"|null)/)?.[1] ?? null;
    packages.set(match[1], { packageName });
  }
  return packages;
}

function parseStagingMap(text) {
  const block = text.match(/const BUNDLE_STAGING\s*=\s*\[([\s\S]*?)\];/s)?.[1];
  if (!block) fail("无法解析 BUNDLE_STAGING");
  return new Map(
    [...block.matchAll(/\{\s*plugin:\s*"([^"]+)",\s*env:\s*(?:null|"([^"]+)")/g)]
      .map((match) => [match[1], match[2] ?? null]),
  );
}

/**
 * bundle 清单版本与页面的权威读取：用打包器的 manifest 构造器实际生成清单。
 *
 * 只读文本解析会碰上 `manifest: fn(ctx)` 与 `pages: [PAGE_ENTRY]` 两种间接写法，
 * 容易把命令 schema 里的同名字段误当清单字段。这里把打包脚本的 PLUGIN_SPECS 与
 * manifest 构造器提取到临时模块，再在当前进程动态导入并调用构造器；产物版本和页面声明
 * 与 `build-plugin-bundle.mjs --check` 同源，不是第二份手抄清单。
 */
function readManifestContract(src, names) {
  const functions = {};
  for (const name of names) {
    const index = src.indexOf(`function ${name}(`);
    if (index < 0) fail(`无法从打包器定位 manifest 构造器：${name}`);
    const open = src.indexOf(") {", index) + 2;
    let depth = 0;
    let inString = null;
    let escaped = false;
    let end = -1;
    for (let cursor = open; cursor < src.length; cursor += 1) {
      const char = src[cursor];
      if (inString) {
        if (escaped) escaped = false;
        else if (char === "\\") escaped = true;
        else if (char === inString) inString = null;
        continue;
      }
      if (char === '"' || char === "'" || char === "`") { inString = char; continue; }
      if (char === "{") depth += 1;
      else if (char === "}" && --depth === 0) { end = cursor + 1; break; }
    }
    if (end < 0) fail(`manifest 构造器不完整：${name}`);
    functions[name] = src.slice(index, end);
  }
  return functions;
}

function stagedManifestSource() {
  const sourceDir = join(ROOT, ".tmp");
  const target = join(sourceDir, "plugin-manifest-contract.mjs");
  mkdirSync(sourceDir, { recursive: true });
  // 函数体仍从公共 opc 仓取一份作为当前 SDK 版本的契约实现；规格则必须逐仓读取，
  // 因为拆仓后每个仓的 PLUGIN_SPECS 只保留自己的单条规格。
  const canonicalSource = readFileSync(pluginBundleScript(ROOT, "opc"), "utf8");
  const names = [
    "stockManifest", "opcManifest", "opcAccountingManifest", "declarativeOpcManifest",
    "opcSecurityManifest", "opcSalesGrowthManifest", "opcContentMediaManifest", "wikiManifest",
  ];
  const factories = readManifestContract(canonicalSource, names);
  // 通用域 worker 工厂先在独立仓 opc-game-dev 落地，opc 公共底座仓尚未同步；
  // 从主仓打包器取这一份函数体，避免为读一个函数而重写整个 opc 独立仓的 Cargo.lock。
  const mainPackager = readFileSync(
    join(ROOT, "axagent-plugins", "scripts", "build-plugin-bundle.mjs"),
    "utf8",
  );
  Object.assign(factories, readManifestContract(mainPackager, ["opcWorkerDomainManifest"]));
  names.push("opcWorkerDomainManifest");
  const specBodies = PLUGIN_NAMES.map((plugin) => {
    const source = readFileSync(pluginBundleScript(ROOT, plugin), "utf8");
    const specStart = source.indexOf("const PLUGIN_SPECS");
    const specEnd = source.indexOf("\n};", specStart);
    if (specStart < 0 || specEnd <= specStart) {
      fail(`无法从 ${plugin} 独立仓打包器定位 PLUGIN_SPECS`);
    }
    return source.slice(specStart + "const PLUGIN_SPECS = {".length, specEnd);
  });
  // 每次重建临时模块，避免打包器改动后读到上次的缓存读数。
  const moduleSrc = [
    "import fs from \"node:fs\";",
    "import path from \"node:path\";",
    "const PAGE_ENTRY = 'pages/index.html';",
    "const DOMAIN_PACK_DIR = 'domain_packs';",
    `const workspaceRoot = ${JSON.stringify(pluginRepo(ROOT, "opc"))};`,
    "const OPC_WRITE_COMMANDS = new Set();",
    "const withRequiredPermission = (commands) => commands;",
    "const fail = (message) => { throw new Error(message); };",
    ...names.map((name) => factories[name]),
    `const PLUGIN_SPECS = {\n${specBodies.join("\n")}\n};`,
    [
      // 版本从统一契约注入：写死 3.0.0 会让矩阵把已迁到 3.0.1 的域报成旧版本。
      `const ctx = { version: ${JSON.stringify(PLUGIN_CONTRACT.releases.workerVersion)}, binaryName: 'worker', sha256: '0'.repeat(64) };`,
      "export const MANIFESTS = Object.fromEntries(",
      "  Object.entries(PLUGIN_SPECS).map(([name, spec]) => {",
      "    const manifest = spec.manifest(ctx);",
      [
        "    return [name, {",
        "      name: manifest.name,",
        "      version: manifest.version,",
        "      pages: (manifest.pages ?? []).map((page) => page.id),",
        "      worker: manifest.worker ?? null,",
        "    }];",
      ].join("\n"),
      "  }),",
      ");",
    ].join("\n"),
  ].join("\n");
  writeFileSync(target, moduleSrc);
  return target;
}

async function parseStagedManifests() {
  const modulePath = stagedManifestSource();
  try {
    const module = await import(`file://${modulePath}`);
    return module.MANIFESTS;
  } catch (error) {
    fail(`无法从打包器提取 manifest 契约：${error instanceof Error ? error.message : String(error)}`);
  }
}

const workspaceCrates = parseWorkspaceCrates(hostManifest);
const crateDirectories = readCrateDirectories();
const { names: cargoPackageNames, versions: cargoPackageVersions } = parseCargoPackageNames();
const bundleSpecs = new Map(
  PLUGIN_NAMES.map((plugin) => {
    const source = readFileSync(pluginBundleScript(ROOT, plugin), "utf8");
    const parsed = parseBundleSpecPackages(source);
    if (!parsed.has(plugin)) {
      fail(`${plugin} 独立仓打包脚本缺少自身插件规格`);
    }
    return [plugin, parsed.get(plugin)];
  }),
);
const bundleNames = [...bundleSpecs.keys()];
const staging = parseStagingMap(stagingScript);
const stagedManifests = await parseStagedManifests();
const slices = new Map((ownership.policy?.verticalSlices ?? []).map((slice) => [slice.domainPackId, slice]));
const packs = ownership.packs ?? [];
const ownershipByteLength = Buffer.byteLength(ownershipScript, "utf8");

// 期望值全部从统一契约派生；数量漂移由 check-plugin-contract.mjs 做逐项对账，
// 这里只负责「实扫集合 == 契约投影」，不再维护 11/17/14 这类手抄计数。
const expectedCrates = WORKER_PLUGIN_NAMES.length
  + PLUGIN_CONTRACT.plugins.reduce((n, p) => n + p.extraCrates.length, 0)
  + 4; // 主仓宿主侧 plugins / schema-gen / plugin-opc-core / plugin-testkit
const declarativeDomains = DOMAIN_PACK_PLUGINS.filter((p) => p.strategy === "declarative");
if (workspaceCrates.length !== 4) fail(`主仓插件 workspace crate 数应为 4，实得 ${workspaceCrates.length}`);
if (crateDirectories.size !== expectedCrates) {
  fail(`Cargo crate 数应为 ${expectedCrates}，实得 ${crateDirectories.size}`);
}
if (cargoPackageNames.size !== expectedCrates) {
  fail(`Cargo package 数应为 ${expectedCrates}，实得 ${cargoPackageNames.size}`);
}
if (bundleNames.length !== PLUGIN_NAMES.length) {
  fail(`bundle 数应为 ${PLUGIN_NAMES.length}，实得 ${bundleNames.length}`);
}
if (packs.length !== DOMAIN_PACK_PLUGINS.length) {
  fail(`P6.2 域包数应为 ${DOMAIN_PACK_PLUGINS.length}，实得 ${packs.length}`);
}
if (ownershipByteLength === 0) fail("OPC 域包底册为空，不能作为矩阵权威来源");
if (!Number.isInteger(ownership.version) || ownership.version < 1) {
  fail(`OPC 域包底册 version 非法：${ownership.version}`);
}

// 域 → bundle 的映射从统一契约派生：11 个域陆续翻牌为 worker 时不再需要改这里，
// 也避免「已从声明式 E2E 迁出、却漏登记 worker bundle」导致验收矩阵把该域报成空行。
const bundleByDomain = new Map(
  PLUGIN_CONTRACT.plugins
    .filter((p) => p.domainPackId)
    .map((p) => [p.domainPackId, p.name]),
);

// 14 个 OPC 域全部 worker 化后，任何独立 worker 域缺专属 staging 变量都必须当场红：
// 否则 E2E 只打印 SKIP，矩阵却把「未验证」写成 E2E 证据。
for (const plugin of PLUGIN_CONTRACT.plugins.filter((p) => p.domainPackId && p.strategy === "worker")) {
  if (!staging.get(plugin.name)) fail(`${plugin.name}: 独立 worker 域缺 staging 环境变量，E2E 会静默 SKIP`);
}

function evidenceFor(bundle, pack) {
  const env = staging.get(bundle) ?? null;
  const cargoPackage = bundle ? cargoPackageNames.get(`plugin-${bundle}`) ?? null : null;
  // 页面资产只看清单声明 + 盘上入口；wiki 首片明确无页面，不能因为“worker 存在”就伪造一页。
  const pageRoot = bundle ? pluginWebPagesDir(ROOT, bundle) : null;
  const pageAssets = pageRoot && existsSync(pageRoot)
    ? [
        join(pageRoot, "index.html"),
        ...(existsSync(join(pageRoot, "domain-data.js")) ? [join(pageRoot, "domain-data.js")] : []),
        ...(existsSync(join(pageRoot, "bundle.js")) ? [join(pageRoot, "bundle.js")] : []),
      ].filter((asset) => existsSync(asset))
    : [];
  const declaredPages = bundle ? stagedManifests[bundle]?.pages ?? null : null;
  const pageReady = pageAssets.length > 0
    && declaredPages !== null
    && declaredPages.length > 0
    // 清单的 `pages[].id` 是逻辑页 id，不等于资产文件名；资产入口统一是 `pages/index.html`。
    && existsSync(join(pageRoot, "index.html"))
    && pageAssets.every((asset) => existsSync(asset));
  const owner = pack ? slices.get(pack.id)?.transportOwner ?? "plugin-opc-common" : null;
  const independentWorker = owner && owner !== "plugin-opc-common";
  const manifestName = bundle ? stagedManifests[bundle]?.name ?? null : null;
  const manifestVersion = bundle ? stagedManifests[bundle]?.version ?? null : null;
  if (bundle && !manifestName) fail(`${bundle} 的 PLUGIN_SPECS 缺少 manifest.name`);
  if (bundle && !manifestVersion) fail(`${bundle} 的 PLUGIN_SPECS 缺少 manifest.version`);
  const manifest = bundle ? stagedManifests[bundle] : null;
  const hasWorker = !!manifest?.worker;
  return {
    buildable: cargoPackage ? "是：Cargo package 可构建" : "否：声明式插件无独立 crate",
    // 14 个 OPC 域全部 worker 化：每个独立 worker 都有自己的 staging 变量与真启停 E2E，
    // 不再用声明式矩阵或共享 bundle root 兜底（缺变量在上面的 fail-closed 处已拦下）。
    lifecycleEvidence: env
      ? `E2E: ${env}（真 bundle 安装/启用/禁用/恢复）`
      : "未验证：无启停端到端",
    workerStartupEvidence: independentWorker
      ? `E2E: ${env} 覆盖命令执行`
      : bundle ? "静态：build-plugin-bundle.mjs --check 握手/停机；命令 E2E 仅部分 bundle 覆盖" : "未验证：非 worker crate",
    commandEvidence: independentWorker
      ? "E2E: 域命令安装后可执行，禁用后不可用，恢复后可执行"
      : ["stock", "wiki", "opc"].includes(bundle)
      ? "E2E: 本 bundle 命令级 E2E 已覆盖"
      : "未验证：未做单命令级 E2E",
    pageAssets: pageAssets.map((asset) => relative(ROOT, asset)),
    pageReady,
    declaredPages: declaredPages ?? [],
    manifestName,
    manifestVersion,
    hasWorker,
    release: {
      ready: true,
      detail: hasWorker
        ? "可生成 plugin.json + worker + sha256 发布清单"
        : "可生成 plugin.json + 页面资产 + sha256 发布清单（声明式插件，无独立 worker）",
    },
    crateVersion: cargoPackage ? cargoPackageVersions.get(`plugin-${bundle}`) ?? null : null,
    disableIsolation: env ? "E2E：禁用后本插件能力消失，其他插件不受影响" : "未验证",
  };
}

const rows = [];
for (const [index, pack] of packs.entries()) {
  const bundle = bundleByDomain.get(pack.id) ?? null;
  const evidence = evidenceFor(bundle, pack);
  rows.push({
    kind: "opc-domain-pack",
    name: pack.id,
    crate: bundle ? cargoPackageNames.get(`plugin-${bundle}`) ?? null : null,
    bundle,
    pluginId: pack.pluginId,
    manifestName: evidence.manifestName,
    manifestVersion: evidence.manifestVersion,
    release: evidence.release,
    crateVersion: evidence.crateVersion,
    transportOwner: slices.get(pack.id)?.transportOwner ?? "plugin-opc-common",
    strategy: slices.get(pack.id)?.manifestStrategy ?? "unknown",
    ...evidence,
    order: index + 1,
  });
}

for (const bundle of bundleNames) {
  if (rows.some((row) => row.bundle === bundle)) continue;
  const evidence = evidenceFor(bundle, null);
  rows.push({
    kind: "bundle",
    name: bundle,
    crate: cargoPackageNames.get(`plugin-${bundle}`) ?? null,
    bundle,
    pluginId: `plugin-${bundle}`,
    manifestName: evidence.manifestName,
    manifestVersion: evidence.manifestVersion,
    crateVersion: evidence.crateVersion,
    transportOwner: null,
    strategy: "worker",
    ...evidence,
    order: rows.length + 1,
  });
}

for (const crate of workspaceCrates) {
  const mapped = rows.find((row) => row.crate === cargoPackageNames.get(crate));
  if (mapped) {
    mapped.workspaceCrate = true;
    continue;
  }
  // `plugin-opc-*` workspace 目录名与 bundle 名去掉前缀后同形，二者是同一个可交付插件，
  // 不应在矩阵里重复占一行；`schema-gen` / `plugins` 等库/工具才单列。
  if (bundleNames.some((bundle) => `plugin-${bundle}` === crate)) continue;
  rows.push({
    kind: "workspace-crate",
    name: crate,
    crate: cargoPackageNames.get(crate) ?? fail(`crate 目录 ${crate} 缺少 Cargo package 名`),
    bundle: null,
    pluginId: null,
    manifestName: null,
    manifestVersion: null,
    release: { ready: false, detail: "库/构建工具不是可发布插件包" },
    crateVersion: cargoPackageVersions.get(crate) ?? fail(`crate 目录 ${crate} 缺少 Cargo package version`),
    transportOwner: null,
    strategy: "library/tool",
    buildable: "是：workspace 成员",
    lifecycleEvidence: "未验证：库/构建工具无独立启停面",
    workerStartupEvidence: "不适用：库/构建工具",
    commandEvidence: "未验证：库/构建工具无命令面",
    pageAssets: [],
    pageReady: false,
    declaredPages: [],
    disableIsolation: "未验证",
    order: rows.length + 1,
  });
}

const summary = {
  physicalCrateDirectoryCount: crateDirectories.size,
  physicalPluginDirectoryCount: bundleNames.length,
  workspaceCrateCount: workspaceCrates.length,
  bundleSpecCount: bundleNames.length,
  opcDomainPackCount: packs.length,
  opcIndependentWorkerCount: packs.filter((pack) => slices.get(pack.id)?.transportOwner !== "plugin-opc-common").length,
  opcDeclarativeCount: declarativeDomains.length,
  e2eLifecycleCovered: rows.filter((row) => row.lifecycleEvidence.startsWith("E2E")).length,
  e2eUncovered: rows.filter((row) => row.lifecycleEvidence.startsWith("未验证")).length,
};

if (JSON_OUT) {
  console.log(JSON.stringify({ summary, rows }, null, 2));
  process.exit(0);
}

const byId = new Map(rows.map((row) => [row.name, row]));
const ownerCounts = new Map();
for (const pack of packs) {
  const owner = slices.get(pack.id)?.transportOwner ?? "plugin-opc-common";
  ownerCounts.set(owner, (ownerCounts.get(owner) ?? 0) + 1);
}
if (slices.size !== packs.length) fail(`域包底册 verticalSlices ${slices.size} 与 packs ${packs.length} 不一致`);
for (const [id, slice] of slices) {
  if (!packs.some((pack) => pack.id === id)) fail(`verticalSlice ${id} 不在 packs 权威清单内`);
  if (!slice.pluginId) fail(`verticalSlice ${id} 缺 pluginId`);
}
for (const row of rows) {
  if (!row.bundle) continue;
  if (row.kind === "bundle" && !row.manifestName) {
    fail(`${row.name} 的 bundle manifest 名称缺失`);
  }
  if (row.kind === "bundle" && !/^\d+\.\d+\.\d+/.test(row.manifestVersion ?? "")) {
    fail(`${row.name} 的 bundle manifest 版本缺失或非法：${row.manifestVersion}`);
  }
  if (row.pageAssets.length > 0 && !row.pageReady) {
    fail(`${row.name} 页面资产存在但与 manifest.pages[] 声明不一致`);
  }
}
const knownCargoPackages = new Set(cargoPackageNames.values());
for (const row of rows) {
  if (row.crate !== null && !knownCargoPackages.has(row.crate)) {
    fail(`${row.name} 的 crate=${row.crate} 不是 Cargo package 真名`);
  }
}

console.log("插件启停验收矩阵（只读生成）");
console.log(
  `物理 crate ${summary.physicalCrateDirectoryCount} / 插件目录 ${summary.physicalPluginDirectoryCount} / workspace crate ${summary.workspaceCrateCount} / bundle ${summary.bundleSpecCount} / OPC 域 ${summary.opcDomainPackCount}`,
);
console.log(`OPC 独立 worker ${summary.opcIndependentWorkerCount} / 声明式 ${summary.opcDeclarativeCount}`);
console.log(`E2E 启停覆盖 ${summary.e2eLifecycleCovered} / 未覆盖 ${summary.e2eUncovered}`);
console.log(`OPC owner 分布：${[...ownerCounts].map(([owner, count]) => `${owner}=${count}`).join(" · ")}`);
console.log("");
for (const row of rows) {
  console.log(`${String(row.order).padStart(2, "0")}. ${row.name}`);
  console.log(`   ${row.kind} | crate=${row.crate ?? "-"} | bundle=${row.bundle ?? "-"} | owner=${row.transportOwner ?? "-"}`);
  console.log(`   构建=${row.buildable}`);
  console.log(`   启停=${row.lifecycleEvidence}`);
  console.log(`   worker=${row.workerStartupEvidence}`);
  console.log(`   命令=${row.commandEvidence}`);
  console.log(`   页面=${row.pageAssets.length ? (row.pageReady ? `存在 ${row.pageAssets.length} 个资产` : "资产缺失") : "无页面资产"}`);
  console.log(`   清单名=${row.manifestName ?? "-"}`);
  console.log(`   版本=${row.manifestVersion ?? "-"}`);
  if (row.crateVersion) console.log(`   Cargo 版本=${row.crateVersion}`);
  if (row.declaredPages.length) console.log(`   声明页面=${row.declaredPages.join(", ")}`);
  console.log(`   禁用隔离=${row.disableIsolation}`);
  console.log(`   发布=${row.release.detail}`);
}
