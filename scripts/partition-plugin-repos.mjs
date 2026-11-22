#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 将业务插件源码的权威位置从主仓迁到 17 个独立仓。
 *
 * 逐文件审计的口径（2026-11-22 起）：**格式差异不算漂移**。两侧先按独立仓的
 * rustfmt / dprint 配置归一化（`.rs` → rustfmt；`.ts/.js` → dprint），归一后逐字节比较；
 * 少数 dprint 无法幂等归一的文件（既有花括号布局）走显式豁免清单
 * `scripts/lib/plugin-partition-exemptions.json`：逐文件、逐理由，并且仍须通过指定的
 * 强规范化器（当前只有 esbuild --minify）。豁免只放宽**格式**，不放宽内容；豁免项与
 * 「真实改动」都逐条打印，缺失/多出/字节比较一条不少。
 *
 * 默认只做逐文件审计，不写工作树。显式 `--apply` 才会：
 *   1. 删除主仓 17 个业务插件目录与 stock-core；
 *   2. 把 axagent-plugins 的 workspace 收缩为宿主 loader、SDK 与 schema-gen；
 *   3. 再次核对独立仓已承接全部可迁移源码，并验证主仓不再含业务插件 crate。
 *
 * 这是单仓迁移工具，不执行 git remote / push / release。
 * 独立仓根默认 `../AxInvest-plugin-repos`，可用 AXAGENT_PLUGIN_REPOS_ROOT 覆盖。
 *
 * 用法：
 *   node scripts/partition-plugin-repos.mjs
 *   node scripts/partition-plugin-repos.mjs --json
 *   node scripts/partition-plugin-repos.mjs --apply
 */
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import { dirname, extname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { PLUGIN_NAMES, pluginRepo, pluginReposRoot } from "./lib/plugin-repos.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const HOST_ROOT = join(ROOT, "axagent-plugins");
const CRATES_ROOT = join(HOST_ROOT, "crates");
const JSON_OUT = process.argv.includes("--json");
const APPLY = process.argv.includes("--apply");
const KEPT_CRATES = new Set(["plugin-opc-core", "plugin-testkit", "plugins", "schema-gen"]);
const BUSINESS_CRATES = [
  ...PLUGIN_NAMES.map((plugin) => `plugin-${plugin}`),
  "stock-core",
];

const fail = (message) => {
  console.error(`FAIL: ${message}`);
  process.exit(1);
};

/** 只归一化「源码格式」类扩展名；其它文件仍按字节比较。 */
const NORMALIZER_BY_EXT = new Map([
  [".rs", "rustfmt"],
  [".ts", "dprint"],
  [".tsx", "dprint"],
  [".js", "dprint"],
  [".jsx", "dprint"],
  [".mjs", "dprint"],
  [".cjs", "dprint"],
]);
const EXEMPTIONS_PATH = join(ROOT, "scripts", "lib", "plugin-partition-exemptions.json");
const LF = String.fromCharCode(10);

/**
 * 豁免清单：逐文件、逐条理由。`canonicalizer` 必须是受支持的规范化器 ——
 * 豁免只放宽**格式**，不允许整文件跳过比较（那等于删掉检查项）。
 */
function loadExemptions() {
  if (!existsSync(EXEMPTIONS_PATH)) return new Map();
  let parsed;
  try {
    parsed = JSON.parse(readFileSync(EXEMPTIONS_PATH, "utf8"));
  } catch (error) {
    fail(`豁免清单不是合法 JSON（${EXEMPTIONS_PATH}）：${error.message}`);
  }
  const map = new Map();
  for (const item of parsed?.exemptions ?? []) {
    const missing = ["plugin", "file", "canonicalizer", "reason"]
      .filter((key) => typeof item?.[key] !== "string" || !item[key].trim());
    if (missing.length) fail(`豁免清单条目缺少字段 ${missing.join("/")}：${JSON.stringify(item)}`);
    if (!["esbuild-minify"].includes(item.canonicalizer)) {
      fail(`豁免清单条目使用了未实现的规范化器 ${item.canonicalizer}：${item.plugin}/${item.file}`);
    }
    map.set(`${item.plugin}:${item.file}`, item);
  }
  return map;
}

function textHash(text) {
  return createHash("sha256").update(text).digest("hex");
}

/** 从 crate 清单取 edition（两侧各按自己的清单；edition 不同就不该判一致）。 */
const editionCache = new Map();
function crateEdition(manifestPath) {
  if (!editionCache.has(manifestPath)) {
    const edition = existsSync(manifestPath)
      ? readFileSync(manifestPath, "utf8").match(/^edition[ \t]*=[ \t]*"([^"]+)"/m)?.[1] ?? "2021"
      : "2021";
    editionCache.set(manifestPath, edition);
  }
  return editionCache.get(manifestPath);
}

/**
 * 归一化必须用**同一份**配置处理两侧，否则 rustfmt 会按文件位置向上找配置：
 * 主仓副本会命中 `axagent-plugins/rustfmt.toml`（Max + 尾逗号），独立仓没有配置
 * ⇒ 两侧各自套不同规则，永远比不平。这里把独立仓的「有效配置」固化到临时目录：
 * 仓内有 rustfmt.toml 就抄过来，没有就空配置（= rustfmt 默认值，仓内现状）。
 */
let rustfmtConfigDirCache;
function rustfmtConfigDir(configRoot) {
  if (!rustfmtConfigDirCache) {
    const dir = mkdtempSync(join(os.tmpdir(), "axinvest-partition-rustfmt-"));
    let text = "";
    for (const name of ["rustfmt.toml", ".rustfmt.toml"]) {
      const candidate = join(configRoot, name);
      if (existsSync(candidate)) { text = readFileSync(candidate, "utf8"); break; }
    }
    writeFileSync(join(dir, "rustfmt.toml"), text);
    rustfmtConfigDirCache = dir;
  }
  return rustfmtConfigDirCache;
}

function firstLine(text) {
  const value = (text ?? "").trim();
  const index = value.indexOf(LF);
  return index < 0 ? value : value.slice(0, index);
}

/** 去掉 rustfmt `--emit stdout` 打印的首行源文件路径（两侧必然不同）。 */
function stripRustfmtHeader(text) {
  const first = text.indexOf(LF);
  if (first < 0 || !text.slice(0, first).endsWith(":")) return text;
  let rest = text.slice(first + 1);
  if (rest.startsWith(LF)) rest = rest.slice(1);
  return rest;
}

function normalizeRust(file, manifestPath, configRoot) {
  const result = spawnSync(
    "rustfmt",
    [
      "--edition", crateEdition(manifestPath),
      "--config", "skip_children=true",
      "--config-path", rustfmtConfigDir(configRoot),
      "--emit", "stdout",
    ],
    { input: readFileSync(file), encoding: "utf8", windowsHide: true },
  );
  if (result.error) return { ok: false, reason: `rustfmt 无法启动：${result.error.message}` };
  if (result.status !== 0) {
    return { ok: false, reason: `rustfmt 退出码 ${result.status}：${firstLine(result.stderr)}` };
  }
  return { ok: true, text: stripRustfmtHeader(result.stdout) };
}

let dprintRuntimeCache;
/** dprint 运行环境；独立仓没有 dprint.json ⇒ 用 dprint 默认设置（只声明 typescript 插件）。 */
function dprintRuntime() {
  if (dprintRuntimeCache !== undefined) return dprintRuntimeCache;
  const binary = join(ROOT, "node_modules", "dprint", "dprint");
  const pluginWasm = join(ROOT, "node_modules", "@dprint", "typescript", "plugin.wasm");
  if (!existsSync(binary) || !existsSync(pluginWasm)) {
    dprintRuntimeCache = null;
    return null;
  }
  const dir = mkdtempSync(join(os.tmpdir(), "axinvest-partition-dprint-"));
  const configPath = join(dir, "dprint.json");
  // 独立仓没有 dprint.json ⇒ 这是它的「有效配置」：dprint 默认值 + typescript 插件。
  writeFileSync(configPath, JSON.stringify({ plugins: [pluginWasm] }, null, 2));
  dprintRuntimeCache = { binary, configPath, dir };
  return dprintRuntimeCache;
}

function dprintConfigFor(repoRoot) {
  // 独立仓若自带 dprint 配置，两侧都用它；否则用上面的默认配置。
  const configPath = join(repoRoot, "dprint.json");
  return existsSync(configPath) ? { binary: join(ROOT, "node_modules", "dprint", "dprint"), configPath, cwd: repoRoot } : dprintRuntime();
}

function normalizeTypeScript(file, name, repoRoot) {
  const runtime = dprintConfigFor(repoRoot);
  if (!runtime) return { ok: false, reason: "缺少 dprint 或 @dprint/typescript 插件" };
  const result = spawnSync(
    runtime.binary,
    ["fmt", "--config", runtime.configPath, "--stdin", name],
    { input: readFileSync(file), encoding: "utf8", windowsHide: true, cwd: runtime.cwd ?? ROOT },
  );
  if (result.error) return { ok: false, reason: `dprint 无法启动：${result.error.message}` };
  if (result.status !== 0) {
    return { ok: false, reason: `dprint 退出码 ${result.status}：${firstLine(result.stderr)}` };
  }
  return { ok: true, text: result.stdout };
}

function esbuildBinary() {
  const packageName = `@esbuild/${process.platform}-${process.arch}`;
  const exe = process.platform === "win32" ? "esbuild.exe" : "esbuild";
  const binary = join(ROOT, "node_modules", ...packageName.split("/"), "bin", exe);
  return existsSync(binary) ? binary : null;
}

/** 豁免项使用的强规范化器：esbuild --minify 去掉一切格式差异，只留语义等价代码。 */
function canonicalizeWithEsbuild(file, ext) {
  const binary = esbuildBinary();
  if (!binary) return { ok: false, reason: "缺少 esbuild 本机二进制" };
  const loader = ext === ".ts" || ext === ".tsx" ? "ts" : "js";
  const result = spawnSync(
    binary,
    ["--minify", `--loader=${loader}`, "--log-level=error"],
    { input: readFileSync(file), encoding: "utf8", windowsHide: true },
  );
  if (result.error) return { ok: false, reason: `esbuild 无法启动：${result.error.message}` };
  if (result.status !== 0) {
    return { ok: false, reason: `esbuild 退出码 ${result.status}：${firstLine(result.stderr)}` };
  }
  return { ok: true, text: result.stdout };
}

function withExemption(sourceFile, targetFile, ext, rel, context, note) {
  const exemption = context.exemptions.get(`${context.plugin}:${rel}`);
  if (!exemption) return { kind: "different", detail: note ?? undefined };
  const canonical = (file) => exemption.canonicalizer === "esbuild-minify"
    ? canonicalizeWithEsbuild(file, ext)
    : { ok: false, reason: `未实现的规范化器 ${exemption.canonicalizer}` };
  const left = canonical(sourceFile);
  const right = canonical(targetFile);
  if (left.ok && right.ok && textHash(left.text) === textHash(right.text)) {
    context.usedExemptions.add(`${context.plugin}:${rel}`);
    return { kind: "exempt", detail: exemption.canonicalizer, reason: exemption.reason };
  }
  const why = !left.ok || !right.ok ? (!left.ok ? left.reason : right.reason) : "归一后仍有差异";
  return { kind: "different", detail: `豁免 ${exemption.canonicalizer} ${why}` };
}

/**
 * 比较一对内容不同的文件。返回 `{ kind: "normalized" | "exempt" | "different" }`。
 * 归一化失败或归一后仍不同 ⇒ `different`（fail-closed，不静默放行）。
 */
function compareChangedFile(sourceFile, targetFile, rel, context) {
  const ext = extname(rel);
  const normalizer = NORMALIZER_BY_EXT.get(ext);
  const name = rel.split("/").at(-1);
  if (normalizer === "rustfmt") {
    // 两侧统一用独立仓的 edition 与配置：独立仓是格式基准，主仓副本的 edition 可能更旧。
    const left = normalizeRust(sourceFile, context.targetManifest, context.targetRepo);
    const right = normalizeRust(targetFile, context.targetManifest, context.targetRepo);
    if (left.ok && right.ok && textHash(left.text) === textHash(right.text)) {
      return { kind: "normalized", detail: "rustfmt" };
    }
    const failure = !left.ok ? left.reason : (!right.ok ? right.reason : null);
    return withExemption(sourceFile, targetFile, ext, rel, context, failure ? `rustfmt 归一失败（${failure}）` : null);
  }
  if (normalizer === "dprint") {
    const left = normalizeTypeScript(sourceFile, name, context.targetRepo);
    const right = normalizeTypeScript(targetFile, name, context.targetRepo);
    if (left.ok && right.ok && textHash(left.text) === textHash(right.text)) {
      return { kind: "normalized", detail: "dprint" };
    }
    const failure = !left.ok ? left.reason : (!right.ok ? right.reason : null);
    return withExemption(sourceFile, targetFile, ext, rel, context, failure ? `dprint 归一失败（${failure}）` : null);
  }
  return withExemption(sourceFile, targetFile, ext, rel, context, null);
}

function walkFiles(root, options = {}) {
  if (!existsSync(root)) return [];
  const files = [];
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const full = join(root, entry.name);
    const rel = relative(root, full);
    if (entry.isDirectory()) {
      if (options.skip?.has(entry.name)) continue;
      files.push(...walkFiles(full, options).map((file) => join(entry.name, file)));
    } else if (entry.isFile()) {
      files.push(rel);
    }
  }
  return files.sort();
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function compareTree(source, target, options = {}) {
  const skip = new Set(["target", "dist", "node_modules", ".git"]);
  // Cargo.toml 在独立仓会被改写为仓内 path 依赖，单独比较 package 名与版本，
  // 不把生成器的预期差异误报为源码漂移。
  const sourceFiles = walkFiles(source, { skip }).filter((file) => file !== "Cargo.toml");
  const targetFiles = walkFiles(target, { skip }).filter((file) => file !== "Cargo.toml");
  const sourceSet = new Set(sourceFiles);
  const targetSet = new Set(targetFiles);
  const missing = sourceFiles.filter((file) => !targetSet.has(file));
  const extra = targetFiles.filter((file) => !sourceSet.has(file));
  const normalized = [];
  const exempt = [];
  const changed = [];
  for (const file of sourceFiles) {
    if (!targetSet.has(file)) continue;
    if (sha256(join(source, file)) === sha256(join(target, file))) continue;
    const outcome = compareChangedFile(join(source, file), join(target, file), file, options.context);
    if (outcome.kind === "normalized") normalized.push({ file, detail: outcome.detail });
    else if (outcome.kind === "exempt") exempt.push({ file, detail: outcome.detail, reason: outcome.reason });
    else changed.push(file);
  }
  return {
    sourceFiles: sourceFiles.length,
    targetFiles: targetFiles.length,
    missing,
    extra,
    changed,
    normalized,
    exempt,
  };
}

function compareManifest(source, target) {
  if (!existsSync(source) || !existsSync(target)) {
    return {
      sourcePackage: existsSync(source) ? "present" : null,
      targetPackage: existsSync(target) ? "present" : null,
      sourceVersion: null,
      targetVersion: null,
      comparable: false,
    };
  }
  const text = readFileSync(source, "utf8");
  const targetText = readFileSync(target, "utf8");
  const inheritedVersion = text.match(/^version\.workspace\s*=\s*true\s*$/m)
    ? readFileSync(join(HOST_ROOT, "Cargo.toml"), "utf8")
      .match(/^\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m)?.[1] ?? null
    : null;
  return {
    sourcePackage: text.match(/^\[package\][\s\S]*?^name\s*=\s*"([^"]+)"/m)?.[1] ?? null,
    targetPackage: targetText.match(/^\[package\][\s\S]*?^name\s*=\s*"([^"]+)"/m)?.[1] ?? null,
    sourceVersion: text.match(/^\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m)?.[1] ?? inheritedVersion,
    targetVersion: targetText.match(/^\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m)?.[1] ?? null,
    comparable: true,
  };
}

function validateWorkspaceMembers() {
  const manifest = readFileSync(join(HOST_ROOT, "Cargo.toml"), "utf8");
  const block = manifest.match(/\[workspace\][\s\S]*?members\s*=\s*\[([\s\S]*?)\]/)?.[1];
  if (!block) fail("无法解析 axagent-plugins workspace members");
  return [...block.matchAll(/"crates\/([^"]+)"/g)].map((match) => match[1]);
}

function expectedMembersAfterPartition() {
  return [...KEPT_CRATES].sort();
}

function validateTargets() {
  const checks = [];
  const exemptions = loadExemptions();
  const usedExemptions = new Set();
  for (const plugin of PLUGIN_NAMES) {
    const repo = pluginRepo(ROOT, plugin);
    if (!existsSync(join(repo, ".git"))) fail(`外置仓缺失或不是 Git 仓：${repo}`);
    const source = join(CRATES_ROOT, `plugin-${plugin}`);
    const target = join(repo, `plugin-${plugin}`);
    if (existsSync(source) && existsSync(target)) {
      const tree = compareTree(source, target, {
        context: {
          plugin,
          sourceManifest: join(source, "Cargo.toml"),
          targetManifest: join(target, "Cargo.toml"),
          targetRepo: pluginRepo(ROOT, plugin),
          exemptions,
          usedExemptions,
        },
      });
      const manifest = compareManifest(
        join(source, "Cargo.toml"),
        join(target, "Cargo.toml"),
      );
      // 承接判定看**包身份**与源码内容，不看版本号：主仓副本的版本来自主仓
      // `[workspace.package]`（宿主应用版本线），独立仓版本才是插件发布坐标
      // （四处一致门禁在 17 仓内各自跑）。副本版本落后只作提示，见 versionDrift。
      const packageMatches = !manifest.comparable || manifest.sourcePackage === manifest.targetPackage;
      const versionDrift = manifest.comparable && manifest.sourceVersion !== manifest.targetVersion
        ? `${manifest.sourceVersion} -> ${manifest.targetVersion}`
        : null;
      checks.push({
        plugin,
        source: relative(ROOT, source),
        target: relative(ROOT, target),
        tree,
        manifest,
        packageMatches,
        versionDrift,
        ok: packageMatches
          && tree.missing.length === 0
          && tree.changed.length === 0
          && tree.extra.length === 0,
      });
    }
  }
  const stockSource = join(CRATES_ROOT, "stock-core");
  const stockTarget = join(pluginRepo(ROOT, "stock"), "stock-core");
  if (existsSync(stockSource) && existsSync(stockTarget)) {
    const tree = compareTree(stockSource, stockTarget, {
      context: {
        plugin: "stock-core",
        sourceManifest: join(stockSource, "Cargo.toml"),
        targetManifest: join(stockTarget, "Cargo.toml"),
        targetRepo: pluginRepo(ROOT, "stock"),
        exemptions,
        usedExemptions,
      },
    });
    const manifest = compareManifest(
      join(stockSource, "Cargo.toml"),
      join(stockTarget, "Cargo.toml"),
    );
    const packageMatches = !manifest.comparable || manifest.sourcePackage === manifest.targetPackage;
    const versionDrift = manifest.comparable && manifest.sourceVersion !== manifest.targetVersion
      ? `${manifest.sourceVersion} -> ${manifest.targetVersion}`
      : null;
    checks.push({
      plugin: "stock-core",
      source: relative(ROOT, stockSource),
      target: relative(ROOT, stockTarget),
      tree,
      manifest,
      packageMatches,
      versionDrift,
      ok: packageMatches
        && tree.missing.length === 0
        && tree.changed.length === 0
        && tree.extra.length === 0,
    });
  }
  // 未被任何文件命中的豁免条目属于过期条目 —— 显式打印，避免「豁免清单只增不减」。
  const staleExemptions = [...exemptions.keys()].filter((key) => !usedExemptions.has(key));
  return { checks, staleExemptions };
}

function runBundleChecks() {
  const failures = [];
  for (const plugin of PLUGIN_NAMES) {
    const result = spawnSync(
      process.execPath,
      [join(pluginRepo(ROOT, plugin), "scripts", "build-plugin-bundle.mjs"), "--plugin", plugin, "--check"],
      {
        cwd: pluginRepo(ROOT, plugin),
        encoding: "utf8",
        env: { ...process.env, CARGO_INCREMENTAL: "0" },
      },
    );
    if (result.status !== 0) {
      failures.push({
        plugin,
        status: result.status,
        detail: `${result.stdout ?? ""}${result.stderr ?? ""}`.trim().split("\n").slice(-8).join("\n"),
      });
    }
  }
  return failures;
}

function rewriteWorkspace() {
  const manifestPath = join(HOST_ROOT, "Cargo.toml");
  const text = readFileSync(manifestPath, "utf8");
  const members = expectedMembersAfterPartition()
    .map((member) => `  "crates/${member}",`)
    .join("\n");
  const next = text.replace(
    /members\s*=\s*\[[\s\S]*?\]/,
    `members = [\n${members}\n]`,
  );
  // 幂等：members 已是分区后形态就直接返回。早期版本在此 fail，会让 --apply 先删目录再抛错，
  // 留下「目录已删、清单没改」的半完成树；现在把改清单放在删目录之前，且重跑不再报错。
  if (next === text) return false;
  writeFileSync(manifestPath, next);
  return true;
}

function main() {
  if (!existsSync(HOST_ROOT)) fail(`主仓插件目录不存在：${HOST_ROOT}`);
  const repositoriesRoot = pluginReposRoot(ROOT);
  if (repositoriesRoot === HOST_ROOT || repositoriesRoot.startsWith(`${ROOT}${sep}`)) {
    // 外置仓必须在主仓之外，否则删除业务目录会碰到独立仓的源码。
    fail(`外置插件仓根必须位于主仓之外：${repositoriesRoot}`);
  }
  const workspaceMembers = validateWorkspaceMembers();
  const { checks, staleExemptions } = validateTargets();
  const failedChecks = checks.filter((check) => !check.ok);
  const remainingBusinessDirs = BUSINESS_CRATES
    .filter((name) => existsSync(join(CRATES_ROOT, name)))
    .sort();

  if (APPLY) {
    if (failedChecks.length > 0) {
      fail(`独立仓尚未完整承接源码，拒绝 --apply：${failedChecks.map((c) => c.plugin).join("、")}`);
    }
    // 先落清单、再删目录：清单写入失败时业务目录仍完整，不会半途而废。
    rewriteWorkspace();
    for (const name of remainingBusinessDirs) {
      rmSync(join(CRATES_ROOT, name), { recursive: true, force: true });
    }
  }

  const finalWorkspaceMembers = validateWorkspaceMembers();
  const finalBusinessDirs = BUSINESS_CRATES.filter((name) => existsSync(join(CRATES_ROOT, name)));
  const expectedMembers = expectedMembersAfterPartition();
  const workspaceOk = JSON.stringify([...finalWorkspaceMembers].sort()) === JSON.stringify(expectedMembers);
  // 审计模式不要求目录已删除（那正是 `--apply` 的动作）；只有 apply 模式才校验物理归零。
  const physicalOk = APPLY ? finalBusinessDirs.length === 0 : true;
  const bundleFailures = APPLY ? runBundleChecks() : [];

  const result = {
    mode: APPLY ? "apply" : "audit",
    repositoriesRoot,
    workspaceBefore: workspaceMembers,
    workspaceAfter: finalWorkspaceMembers,
    expectedWorkspaceMembers: expectedMembers,
    businessCratesBefore: remainingBusinessDirs,
    businessCratesAfter: finalBusinessDirs,
    repositoryChecks: checks.map((check) => ({
      plugin: check.plugin,
      ok: check.ok,
      sourceFiles: check.tree.sourceFiles,
      targetFiles: check.tree.targetFiles,
      missing: check.tree.missing,
      extra: check.tree.extra,
      changed: check.tree.changed,
      normalized: check.tree.normalized,
      exempt: check.tree.exempt,
      versionDrift: check.versionDrift,
      manifest: check.manifest,
    })),
    staleExemptions,
    bundleFailures,
    ok: failedChecks.length === 0 && workspaceOk && physicalOk && bundleFailures.length === 0,
  };

  if (JSON_OUT) {
    console.log(JSON.stringify(result, null, 2));
  } else {
    console.log(`模式：${result.mode}`);
    console.log(`外置仓根：${repositoriesRoot}`);
    console.log(`workspace：${workspaceMembers.join(", ")} -> ${finalWorkspaceMembers.join(", ")}`);
    console.log(`主仓业务插件目录：${remainingBusinessDirs.length} -> ${finalBusinessDirs.length}`);
    for (const check of result.repositoryChecks) {
      console.log(
        `${check.ok ? "PASS" : "FAIL"} ${check.plugin}: ${check.sourceFiles} files -> ${check.targetFiles}`
          + `，缺失 ${check.missing.length} / 多出 ${check.extra.length}`
          + ` / 真实改动 ${check.changed.length}`
          + ` / 格式归一 ${check.normalized?.length ?? 0}`
          + ` / 豁免归一 ${check.exempt?.length ?? 0}`,
      );
      if (check.missing.length) console.log(`  缺失：${check.missing.join(", ")}`);
      if (check.extra.length) console.log(`  多出：${check.extra.join(", ")}`);
      if (check.changed.length) console.log(`  真实改动：${check.changed.join(", ")}`);
      if (check.versionDrift) {
        console.log(`  主仓副本版本落后：${check.versionDrift}（副本待删除；插件发布版本以独立仓为准）`);
      }
      if (check.normalized?.length) {
        const kinds = [...new Set(check.normalized.map((item) => item.detail))].join("/");
        console.log(`  格式归一（${kinds}）：${check.normalized.map((item) => item.file).join(", ")}`);
      }
      for (const item of check.exempt ?? []) {
        console.log(`  豁免归一（${item.detail}）：${item.file}`);
        console.log(`    理由：${item.reason}`);
      }
    }
    for (const key of staleExemptions) {
      console.log(`  未命中的豁免（文件已一致或已迁移，应删除）：${key}`);
    }
    if (bundleFailures.length) {
      for (const failure of bundleFailures) {
        console.log(`FAIL bundle ${failure.plugin} (exit ${failure.status})\n${failure.detail}`);
      }
    }
    console.log(result.ok ? (APPLY ? "PLUGIN_REPOS_PARTITION_OK" : "PLUGIN_REPOS_PARTITION_AUDIT_OK") : "PLUGIN_REPOS_PARTITION_FAIL");
  }
  if (!result.ok) process.exit(1);
}

main();
