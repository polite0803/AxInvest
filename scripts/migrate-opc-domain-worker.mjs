#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 单个 OPC 域从「声明式」翻牌为「独立 worker」的机械改动（阶段 2/3 复用）。
 *
 * 只改有唯一权威的五处事实源：统一契约、域包底册、域包 YAML、打包器 PLUGIN_SPECS
 * 静态字面量、plugins workspace 的 exclude；随后调用 generate-opc-worker-source.mjs
 * 生成 crate 源码。命令集从底册原 capabilities 机械派生并写入契约，之后契约就是命令面的
 * 唯一权威（底册/YAML/manifest 都从契约对账）。
 *
 * 本脚本刻意不做：构建 bundle、同步独立仓、发布。每域翻牌后必须跑定点验收再进下一域。
 * 已翻牌的域直接拒绝，避免重复追加 capabilities 污染命令面。
 *
 * 用法：node scripts/migrate-opc-domain-worker.mjs --domain ai_research
 */
import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const domain = process.argv[process.argv.indexOf("--domain") + 1];
const fail = (message) => {
  console.error(`FAIL: ${message}`);
  process.exit(1);
};
if (!domain || domain.startsWith("--")) fail("需要 --domain <域包 id>");

const CONTRACT_PATH = join(ROOT, "scripts", "lib", "plugin-contract.json");
const REGISTRY_PATH = join(ROOT, "axagent-plugins", "scripts", "opc-domain-pack-plugins.json");
const YAML_PATH = join(ROOT, "config", "opc", "domain_packs", domain, "manifest.yaml");
const PACKAGER_PATH = join(ROOT, "axagent-plugins", "scripts", "build-plugin-bundle.mjs");
const CARGO_PATH = join(ROOT, "axagent-plugins", "Cargo.toml");
const GENERATOR = join(ROOT, "scripts", "generate-opc-worker-source.mjs");

const read = (path) => readFileSync(path, "utf8");
const write = (path, text) => writeFileSync(path, text);
const contract = JSON.parse(read(CONTRACT_PATH));
const registry = JSON.parse(read(REGISTRY_PATH));
const entry = contract.plugins.find((p) => p.domainPackId === domain);
const slice = registry.policy?.verticalSlices?.find((s) => s.domainPackId === domain);
const pack = registry.packs?.find((p) => p.id === domain);
if (!entry || !slice || !pack) fail(`契约 / 底册缺少域 ${domain}`);
if (entry.strategy === "worker") fail(`域 ${domain} 已是 worker，拒绝重复翻牌`);
if (entry.strategy !== "declarative") fail(`域 ${domain} 的 strategy 非法：${entry.strategy}`);

const plugin = entry.name;
const pascal = (command) => command.split("_").map((part) => part[0].toUpperCase() + part.slice(1)).join("");
const hostTools = (pack.capabilities ?? []).filter((cap) => cap.startsWith("tool:"));
const toolCommands = hostTools.map((cap) => `opc_${domain}_${cap.slice("tool:".length)
  .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
  .toLowerCase()}`);
const runCommand = `opc_${domain}_run_automation_rules`;
const commands = [...toolCommands, runCommand];
if (new Set(commands).size !== commands.length) fail(`域 ${domain} 生成的命令名有重复`);
const capIds = commands.map((command) => `tool:${pascal(command)}`);

// ① 契约：命令面唯一权威。版本沿用 worker 发布线，页面资产补 bundle.js。
entry.strategy = "worker";
entry.version = contract.releases.workerVersion;
entry.crate = `plugin-${plugin}`;
entry.packageName = `axagent-plugin-${plugin}`;
entry.sdk = ["plugin-opc-core", "plugin-testkit"];
entry.extraCrates = [];
entry.manifestFactory = "opcWorkerDomainManifest";
entry.transportOwner = plugin;
entry.manifestStrategy = "independent-worker";
entry.commands = commands;
entry.pageAssets = ["index.html", "domain-data.js", "bundle.js"];
write(CONTRACT_PATH, `${JSON.stringify(contract, null, 2)}\n`);

// ② 底册：owner / 形态 / capabilities 与契约同步。
slice.transportOwner = plugin;
slice.manifestStrategy = "independent-worker";
slice.reason = "域专属 worker 声明完整命令集，经 tool.set / opc.store 接缝复用宿主实现，不复制工具逻辑";
pack.capabilities = [...hostTools, ...capIds];
write(REGISTRY_PATH, `${JSON.stringify(registry, null, 2)}\n`);

// ③ YAML：域包逻辑清单的 capabilities 也要登记域命令，保持与底册一致。
const yaml = read(YAML_PATH);
const yamlAddition = [
  "",
  "  # 域专属 worker 命令：由该域 Rust worker 声明，经 tool.set / opc.store 接缝复用宿主工具实现。",
  ...capIds.flatMap((id) => [
    "  - source: tool",
    "    capability_type: general_tool",
    `    predicate: {kind: id_exact, id: "${id}"}`,
  ]),
].join("\n");
write(YAML_PATH, `${yaml.replace(/\s+$/, "")}${yamlAddition}`);

// ③b 主仓 crate 的逻辑清单：bundle 的 --check 与独立仓门禁都逐字段对账它，
// 旧声明式版本的 transportOwner=plugin-opc-common、commands=[]、calls 只有 opc.store。
const logicalPath = join(ROOT, "axagent-plugins", "crates", `plugin-${plugin}`, "domain_packs", `${domain}.json`);
const logical = JSON.parse(read(logicalPath));
logical.transportOwner = plugin;
logical.capabilities = [...hostTools, ...capIds];
logical.commands = commands;
logical.calls = ["opc.store", "tool.set"];
write(logicalPath, `${JSON.stringify(logical, null, 2)}\n`);

// ④ 打包器静态字面量：多个脚本按文本正则读取，必须保持字面量形式，不能运行时投影。
const packager = read(PACKAGER_PATH);
const blockPattern = new RegExp(`(?:  // [^\\n]*\\n)*  "${plugin}": \\{[\\s\\S]*?\\n  \\},\\n`);
if (!blockPattern.test(packager)) fail(`打包器 ${plugin} 条目形态无法解析`);
const replacement = [
  `  "${plugin}": {`,
  `    packageName: "axagent-plugin-${plugin}",`,
  `    webDir: path.join("crates", "plugin-${plugin}", "web"),`,
  "    uiSchema: null,",
  "    commands: [",
  ...commands.map((command) => `      "${command}",`),
  "    ],",
  `    manifest: opcWorkerDomainManifest("${plugin}", ${JSON.stringify(entry.displayName)}, "${domain}"),`,
  "  },",
  "",
].join("\n");
// 条目前紧邻的注释一并替换：security 旧条目上方写着「没有 worker 命令」，
// 翻牌后若留下就是误导性文档。
const nextPackager = packager.replace(blockPattern, replacement);
write(PACKAGER_PATH, nextPackager);

// ⑤ workspace exclude：worker crate 在主仓只作模板源，不进 members。
const cargo = read(CARGO_PATH);
const excludeLine = `  "crates/plugin-${plugin}",\n`;
if (!cargo.includes(excludeLine)) {
  const excludeStart = cargo.indexOf("exclude = [\n");
  if (excludeStart < 0) fail("plugins Cargo.toml 缺少 exclude 段");
  const insertAt = cargo.indexOf("\n]", excludeStart);
  if (insertAt < 0) fail("plugins Cargo.toml 的 exclude 段未闭合");
  const lines = cargo.slice(excludeStart + "exclude = [\n".length, insertAt).split("\n").filter(Boolean);
  lines.push(`  "crates/plugin-${plugin}",`);
  lines.sort();
  write(CARGO_PATH, cargo.slice(0, excludeStart) + "exclude = [\n" + lines.join("\n") + cargo.slice(insertAt));
}

// ⑥ 声明式 E2E 矩阵已随 14 域全 worker 退役：启停证据改由每域专属 staging 变量门控的
// worker E2E 提供，验收矩阵从 cargo-all-workspaces.mjs 的 staging 表派生，不再维护 case 表。

// ⑦ crate 源码由唯一生成器落盘。
const generated = spawnSync(process.execPath, [GENERATOR, "--domain", domain], { stdio: "inherit" });
if (generated.status !== 0) fail("worker 源码生成失败");

console.log(`OPC_DOMAIN_MIGRATED ${domain}: ${commands.length} 条命令 / owner=${plugin}`);
