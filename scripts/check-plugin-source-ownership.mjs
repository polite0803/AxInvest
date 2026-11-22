#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 插件源码归属与「物理删除前置条件」只读对账门。
 *
 * 背景（2026-11-22 实测）：17 个插件业务仓已是源码权威，但主仓
 * `axagent-plugins/crates/<exclude 项>` 仍被 `scripts/lib/plugin-build-roots.mjs`
 * 经 `--bootstrap-from-main` 物化为 CI/Release 的 cargo 构建根，且
 * `axagent-plugins/scripts/build-plugin-bundle.mjs` 的页面/域包/清单源码根仍指向主仓。
 * 因此「删掉 18 个 exclude 目录」不是纯清理，直接删会打断桌面插件打包。
 *
 * 本门分两类判定：
 *   1. **硬不变量**：契约数量、宿主 workspace 成员、`.ignore` 与 `exclude` 同源、
 *      过渡模板目录存在性（删链未完成前不得凭空消失）—— 违反即红。
 *   2. **删除前置**：独立仓可用性、partition audit、构建根已切独立仓、打包器源码根已切、
 *      CI 能获取独立仓 —— 只报告，不判红；等条件全部满足后，才允许模板目录归零。
 *
 * 用法：node scripts/check-plugin-source-ownership.mjs [--selftest]
 */
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { PLUGIN_NAMES, pluginRepo, pluginReposRoot } from "./lib/plugin-repos.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SELFTEST = process.argv.includes("--selftest");
const KEPT_CRATES = ["plugin-opc-core", "plugin-testkit", "plugins", "schema-gen"];
const BUSINESS_CRATES = [...PLUGIN_NAMES.map((name) => `plugin-${name}`), "stock-core"];

/** 从 Cargo workspace 的 members / exclude 数组取最后一段目录名。 */
export function parseWorkspaceNames(manifestText, field) {
  const block = manifestText.match(new RegExp(`${field}\\s*=\\s*\\[([\\s\\S]*?)\\]`))?.[1];
  if (!block) return [];
  return [...block.matchAll(/"([^"]+)"/g)]
    .map((match) => match[1].split("/").at(-1))
    .filter(Boolean);
}

/** `.ignore` 里只取显式列出的主仓插件模板项；通配符会连宿主 crate 一起隐藏，必须拒绝。 */
export function parseIgnoredCrates(ignoreText) {
  const entries = ignoreText.split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith("#"))
    .filter((line) => line.startsWith("axagent-plugins/crates/"));
  // 只认 `axagent-plugins/crates/<name>/` 这一层；`.../plugins/bundled/` 等生成物
  // 也属于同一前缀，但不是 workspace exclude 项，不能混进逐项对账。
  return entries
    .filter((line) => line.split("/").length === 4)
    .map((line) => line.replace(/\/+$/, "").split("/").at(-1));
}

/** 硬门：搜索隔离面必须与 workspace exclude 逐项一致，且不得用通配符误伤宿主 crate。 */
export function compareIgnoreWithExcludes(excluded, ignored) {
  const excludeSet = new Set(excluded);
  const ignoreSet = new Set(ignored);
  const missing = [...excludeSet].filter((name) => !ignoreSet.has(name)).sort();
  const extra = [...ignoreSet].filter((name) => !excludeSet.has(name)).sort();
  const wildcards = ignored.filter((name) => name.includes("*"));
  const hostHidden = ignored.filter((name) => KEPT_CRATES.includes(name));
  return { missing, extra, wildcards, hostHidden, ok: !missing.length && !extra.length && !wildcards.length && !hostHidden.length };
}

function readText(relativePath) {
  return readFileSync(join(ROOT, relativePath), "utf8");
}

function selftest() {
  const checks = [];
  const t = (name, ok) => checks.push({ name, ok: ok === true });

  const excluded = ["plugin-stock", "stock-core", "plugin-opc"];
  t("同名逐项对账通过", compareIgnoreWithExcludes(excluded, ["stock-core", "plugin-opc", "plugin-stock"]).ok);
  t("漏项被检出", !compareIgnoreWithExcludes(excluded, ["plugin-stock", "plugin-opc"]).ok);
  t("多出项被检出", !compareIgnoreWithExcludes(excluded, ["plugin-stock", "stock-core", "plugin-opc", "plugin-x"]).ok);
  t("通配符被检出", compareIgnoreWithExcludes(excluded, ["plugin-opc-*", "plugin-stock", "stock-core"]).wildcards.length === 1);
  t("宿主 crate 被隐藏时判红", compareIgnoreWithExcludes(excluded, [...excluded, "plugin-opc-core"]).hostHidden.includes("plugin-opc-core"));
  t("members 解析", JSON.stringify(parseWorkspaceNames('[workspace]\nmembers = [\n "crates/plugins",\n]\n', "members")) === '["plugins"]');
  t(".ignore 注释与生成物不混入", JSON.stringify(parseIgnoredCrates("# note\nnode_modules/\naxagent-plugins/crates/plugin-stock/\n")) === '["plugin-stock"]');

  const failed = checks.filter((check) => !check.ok);
  for (const check of checks) {
    console.log(`  ${check.ok ? "✔" : "✖"} ${check.name}`);
  }
  console.log(`selftest：${checks.length - failed.length} passed / ${failed.length} failed`);
  process.exit(failed.length === 0 ? 0 : 1);
}

function independentReposAvailable() {
  return PLUGIN_NAMES.every((name) => existsSync(join(pluginRepo(ROOT, name), ".git")));
}

function partitionAudit() {
  const result = spawnSync(process.execPath, ["scripts/partition-plugin-repos.mjs", "--json"], {
    cwd: ROOT,
    encoding: "utf8",
    windowsHide: true,
  });
  if (result.error) return { ok: false, reason: result.error.message };
  try {
    const value = JSON.parse(result.stdout ?? "");
    return { ok: value.ok === true, reason: value.ok ? "18 个模板源与独立仓逐文件一致" : "模板源与独立仓仍有文件差异" };
  } catch {
    return { ok: false, reason: `分区审计输出无法解析（exit ${result.status}）` };
  }
}

function prerequisiteReport(excluded, templatesPresent) {
  const buildRoots = readText("scripts/lib/plugin-build-roots.mjs");
  const bundler = readText("axagent-plugins/scripts/build-plugin-bundle.mjs");
  const ciText = ["ci.yml", "ci-split.yml", "release.yml"]
    .filter((name) => existsSync(join(ROOT, ".github", "workflows", name)))
    .map((name) => readText(join(".github", "workflows", name)))
    .join("\n");
  const repos = independentReposAvailable();
  const audit = repos && templatesPresent ? partitionAudit() : null;
  return [
    {
      name: "独立仓可用（17 个 Git 仓）",
      ok: repos,
      detail: repos ? pluginReposRoot(ROOT) : "本机未配置 sibling 仓；CI 不应强依赖",
    },
    {
      name: "分区审计逐文件一致",
      ok: audit?.ok ?? false,
      detail: audit ? audit.reason : "独立仓不可用或模板源已删除，无需审计",
      applicable: Boolean(audit),
    },
    {
      name: "构建根不再从主仓模板物化",
      ok: !buildRoots.includes("--bootstrap-from-main"),
      detail: buildRoots.includes("--bootstrap-from-main")
        ? "scripts/lib/plugin-build-roots.mjs 仍调用 --bootstrap-from-main"
        : "已切换",
    },
    {
      name: "打包器源码根不再指向主仓",
      ok: !bundler.includes("axagent-plugins/crates/"),
      detail: bundler.includes("axagent-plugins/crates/")
        ? "build-plugin-bundle.mjs 仍有主仓模板路径"
        : "已切换",
    },
    {
      name: "CI 能稳定获取独立仓",
      ok: ciText.includes("AXAGENT_PLUGIN_REPOS_ROOT") || ciText.includes("AxInvest-plugin-repos"),
      detail: "需 checkout 或固定路径指向独立仓，才能把权威源当构建输入",
    },
  ];
}

function main() {
  if (SELFTEST) selftest();

  const manifest = readText("axagent-plugins/Cargo.toml");
  const ignored = parseIgnoredCrates(readText(".ignore"));
  const members = parseWorkspaceNames(manifest, "members");
  const excluded = parseWorkspaceNames(manifest, "exclude");
  const gates = [];

  const contract = JSON.parse(readText("scripts/lib/plugin-contract.json"));
  const contractOk = contract.plugins?.length === PLUGIN_NAMES.length
    && PLUGIN_NAMES.every((name) => contract.plugins.some((plugin) => plugin.name === name));
  gates.push({ name: "契约插件数 17", ok: contractOk });

  const membersOk = JSON.stringify([...members].sort()) === JSON.stringify([...KEPT_CRATES].sort());
  gates.push({ name: "宿主 workspace 成员 4 个齐全", ok: membersOk });

  const ignoreCmp = compareIgnoreWithExcludes(excluded, ignored);
  gates.push({ name: ".ignore 与 exclude 逐项同源", ok: ignoreCmp.ok });

  const present = BUSINESS_CRATES.filter((name) => existsSync(join(ROOT, "axagent-plugins", "crates", name)));
  const prerequisites = prerequisiteReport(excluded, present.length > 0);
  const allPrerequisitesMet = prerequisites.filter((item) => item.applicable !== false).every((item) => item.ok);
  const templatesOk = present.length === 0
    ? allPrerequisitesMet
    : present.length === BUSINESS_CRATES.length;
  gates.push({
    name: present.length === 0 ? "模板源已删除且前置全部满足" : "过渡模板源 18 个仍在（构建链未切换前不可删）",
    ok: templatesOk,
  });

  console.log("插件源码权威归属对账（只读）");
  console.log(`契约插件 ${PLUGIN_NAMES.length} / worker ${contract.plugins?.filter((p) => p.strategy === "worker").length} / 域包 ${contract.plugins?.filter((p) => p.domainPackId).length}`);
  console.log(`宿主 workspace 成员：${members.join(", ")}`);
  console.log(`过渡模板源：${present.length}/${BUSINESS_CRATES.length}；exclude ${excluded.length} 项；.ignore ${ignored.length} 项`);
  if (ignoreCmp.missing.length) console.log(`  .ignore 漏项：${ignoreCmp.missing.join(", ")}`);
  if (ignoreCmp.extra.length) console.log(`  .ignore 多项：${ignoreCmp.extra.join(", ")}`);
  if (ignoreCmp.wildcards.length) console.log(`  .ignore 通配符：${ignoreCmp.wildcards.join(", ")}`);
  if (ignoreCmp.hostHidden.length) console.log(`  .ignore 误伤宿主 crate：${ignoreCmp.hostHidden.join(", ")}`);

  console.log("物理删除前置条件（报告项，暂不判红）：");
  for (const item of prerequisites) {
    console.log(`  ${item.ok ? "✔" : "✖"} ${item.name} —— ${item.detail}`);
  }
  const unmet = prerequisites.filter((item) => item.applicable !== false && !item.ok).length;
  console.log(`结论：${unmet === 0 ? "DELETION_READY" : `DELETION_NOT_READY（${unmet} 项前置未满足）`}`);
  console.log("权威源：AxInvest-plugin-repos/<plugin>/；主仓模板源仅作 CI/Release 过渡构建输入，暂不删除。");

  const failed = gates.filter((gate) => !gate.ok);
  for (const gate of failed) {
    console.error(`FAIL: ${gate.name}`);
  }
  if (failed.length) process.exit(1);
  console.log("PLUGIN_SOURCE_OWNERSHIP_OK");
}

main();
