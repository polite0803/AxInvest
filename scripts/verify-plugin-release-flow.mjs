#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 插件发布地址安装链的契约回归。
 *
 * 这不冒充真机 GUI 或 `cargo publish` 验收；它验证四份权威是否仍对齐：
 *   1. 发布生成器只接受 HTTPS `.tar.gz` / `.tgz`，按 os / arch 生成并聚合产物；
 *   2. 前端把直接粘贴的 HTTPS release.json 归一化成 `axagent-release+https://...`；
 *   3. Rust `parse_install_source` 在下载前把它识别为 ReleaseManifestUrl；
 *   4. 安装命令使用 `install_verified` 并把后端错误映射为结构化错误码；
 *   5. 17/17 外置插件仓 bundle 自检通过，发布包由各仓自己的产物生成（`--ci-safe` 只跑 1–4，第 5 项留给本地集成与各仓 CI）。
 */
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chmodSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import {
  PLUGIN_CONTRACT,
  PLUGIN_NAMES,
  pluginBundleDist,
  pluginBundleScript,
  pluginRepo,
} from "./lib/plugin-repos.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const REPO_VERIFY = join(ROOT, "scripts", "generate-plugin-independent-repos.mjs");
const RELEASE_GEN = join(ROOT, "scripts", "generate-plugin-release-manifest.mjs");
const SOURCE_SECURITY = join(ROOT, "src", "lib", "pluginSourceSecurity.ts");
const MANAGER_RS = join(ROOT, "axagent-plugins", "crates", "plugins", "src", "manager.rs");
const PLUGIN_CMD_RS = join(ROOT, "src-tauri", "src", "commands", "plugin.rs");
const GALLERY_TSX = join(ROOT, "src", "components", "chat", "PluginGallery.tsx");
const BUNDLE_SCRIPT = pluginBundleScript(ROOT, "opc");
/** CI 安全面只读主仓打包器；独立仓副本由本地集成入口验证。 */
const MAIN_BUNDLE_SCRIPT = join(ROOT, "axagent-plugins", "scripts", "build-plugin-bundle.mjs");

function run(command, args, cwd = ROOT) {
  const result = spawnSync(command, args, { cwd, encoding: "utf8", windowsHide: true });
  const output = `${result.stdout ?? ""}${result.stderr ?? ""}`.trim();
  if (result.error || result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} 失败：${result.error?.message ?? `exit ${result.status}`}\n${output}`);
  }
  return output;
}

function runExpectFailure(command, args, expected, cwd = ROOT) {
  const result = spawnSync(command, args, { cwd, encoding: "utf8", windowsHide: true });
  const output = `${result.stdout ?? ""}${result.stderr ?? ""}`.trim();
  assert(!result.error, `${command} ${args.join(" ")} 无法执行：${result.error?.message ?? "unknown error"}`);
  assert(result.status !== 0, `${command} ${args.join(" ")} 应失败但退出码为 0`);
  assert(output.includes(expected), `${command} ${args.join(" ")} 失败信息缺少 ${expected}：${output}`);
  return output;
}

function assert(condition, message) {
  if (!condition) throw new Error(`FAIL: ${message}`);
}

/** 直接从发布生成器源码提取默认目录哈希算法，用同一口径核对 release.json。 */
function directoryHash(root) {
  const files = [];
  let hasWorker = false;
  function visit(dir) {
    for (const entry of readdirSync(dir, { withFileTypes: true }).sort((a, b) =>
      a.name < b.name ? -1 : a.name > b.name ? 1 : 0
    )) {
      const full = join(dir, entry.name);
      if (entry.isDirectory()) visit(full);
      else if (entry.isFile()) {
        const name = relative(root, full).split("\\").join("/");
        files.push(name);
        if (name.startsWith("bin/")) hasWorker = true;
      }
      else throw new Error(`发布目录不允许特殊条目：${relative(ROOT, full)}`);
    }
  }
  visit(root);
  if (hasWorker) {
    for (const file of files) {
      if (file.startsWith("bin/")) {
        const full = join(root, file);
        let mode = statSync(full).mode & 0o777;
        if (!(mode & 0o400)) {
          mode |= 0o700;
          chmodSync(full, mode);
        }
      }
    }
  }
  files.sort();
  const hash = createHash("sha256");
  for (const file of files) {
    hash.update(file);
    hash.update(readFileSync(join(root, file)));
  }
  return hash.digest("hex");
}

function verifyReleaseArtifact(bundle, out, artifactUrl, os, arch, legacy = false) {
  const releasePath = join(out, "release.json");
  const archivePath = join(out, `${bundle.split(/[\\/]/).pop()}.tar.gz`);
  assert(existsSync(releasePath), `发布清单不存在：${relative(ROOT, releasePath)}`);
  assert(existsSync(archivePath), `成品包不存在：${relative(ROOT, archivePath)}`);
  const release = JSON.parse(readFileSync(releasePath, "utf8"));
  let artifact;
  if (legacy) {
    assert(!Object.hasOwn(release, "artifacts"), "旧格式 release.json 不应包含 artifacts[]");
    artifact = { os, arch, artifactUrl: release.artifactUrl, sha256: release.sha256 };
  } else {
    assert(Array.isArray(release.artifacts) && release.artifacts.length === 1, "平台格式必须恰好声明一个 artifacts[] 条目");
    artifact = release.artifacts[0];
  }
  assert(/^[0-9a-f]{64}$/.test(artifact.sha256), "release.json sha256 必须是 64 位小写十六进制");
  assert(artifact.artifactUrl === artifactUrl, "release.json artifactUrl 与输入不一致");
  assert(artifact.os === os && artifact.arch === arch, "release.json os / arch 与输入不一致");
  assert(artifactUrl.startsWith("https://"), "成品包地址必须是 HTTPS");
  assert(/\.(tar\.gz|tgz)$/.test(artifactUrl), "成品包地址必须是 .tar.gz / .tgz");
  const staged = join(out, ".verify-stage");
  rmSync(staged, { recursive: true, force: true });
  mkdirSync(staged, { recursive: true });
  run("tar", ["-xzf", archivePath, "-C", staged]);
  // 发布归档固定 0644 权限，目录条目缺少执行位；先补回目录权限再读取内容。
  const dirs = [staged];
  for (const entry of readdirSync(staged, { withFileTypes: true })) {
    if (entry.isDirectory()) dirs.push(join(staged, entry.name));
  }
  for (const dir of dirs) {
    if (existsSync(dir)) chmodSync(dir, 0o700);
  }
  const actual = directoryHash(staged);
  for (const dir of dirs) {
    if (existsSync(dir)) chmodSync(dir, 0o700);
  }
  assert(actual === artifact.sha256, `release.json 哈希与压缩包实际内容不一致：${artifact.sha256} != ${actual}`);
  rmSync(staged, { recursive: true, force: true });
  const manifest = JSON.parse(readFileSync(join(bundle, "plugin.json"), "utf8"));
  assert(release.version === manifest.version, "release.json version 与 plugin.json 不一致");
  return { artifact, version: release.version };
}

function generatePlatformRelease(bundle, out, artifactUrl, os, arch, legacy = false) {
  const args = [
    RELEASE_GEN,
    "--bundle",
    relative(ROOT, bundle),
    "--artifact-url",
    artifactUrl,
    "--os",
    os,
    "--arch",
    arch,
    "--out",
    relative(ROOT, out),
  ];
  if (legacy) args.push("--legacy");
  run(process.execPath, args);
  run(process.execPath, [RELEASE_GEN, "--check", ...args.slice(1)]);
}

function verifyPlatformMerge(plugin, bundle, manifest) {
  const platformRoot = join(ROOT, ".tmp", "plugin-release-flow", "platform-merge", plugin);
  const platforms = [
    ["linux", "x86_64"],
    ["linux", "aarch64"],
    ["macos", "x86_64"],
    ["macos", "aarch64"],
    ["windows", "x86_64"],
    ["windows", "aarch64"],
  ];
  const sources = [];
  for (const [os, arch] of platforms) {
    const out = join(platformRoot, `${os}-${arch}`);
    const artifactUrl = `https://cdn.example.com/axagent/${plugin}-${manifest.version}-${os}-${arch}.tar.gz`;
    generatePlatformRelease(bundle, out, artifactUrl, os, arch);
    sources.push(join(out, "release.json"));
  }

  const mergedOut = join(platformRoot, "merged");
  const mergeArgs = [
    RELEASE_GEN,
    "--merge",
    "--version",
    manifest.version,
    "--out",
    relative(ROOT, mergedOut),
    "--stage-artifacts",
    ...sources.flatMap((source) => ["--artifact", relative(ROOT, source)]),
    ...platforms.flatMap(([os, arch]) => ["--require-platform", `${os}/${arch}`]),
  ];
  run(process.execPath, mergeArgs);
  const merged = JSON.parse(readFileSync(join(mergedOut, "release.json"), "utf8"));
  assert(merged.version === manifest.version, "聚合 release.json version 不一致");
  assert(Array.isArray(merged.artifacts) && merged.artifacts.length === platforms.length, "聚合 release.json 平台数不完整");
  const actualPlatforms = merged.artifacts.map((artifact) => `${artifact.os}/${artifact.arch}`);
  const expectedPlatforms = platforms.map(([os, arch]) => `${os}/${arch}`);
  const sortedPlatforms = [...actualPlatforms].sort((a, b) => a.localeCompare(b));
  assert(
    actualPlatforms.join(",") === sortedPlatforms.join(",")
      && [...expectedPlatforms].sort((a, b) => a.localeCompare(b)).join(",") === sortedPlatforms.join(","),
    `聚合平台顺序或内容错误：${actualPlatforms.join(",")}`,
  );
  for (const artifact of merged.artifacts) {
    assert(/^https:\/\/\S+\.(tar\.gz|tgz)$/.test(artifact.artifactUrl), `聚合产物地址非法：${artifact.artifactUrl}`);
    assert(/^[0-9a-f]{64}$/.test(artifact.sha256), `聚合产物哈希非法：${artifact.os}/${artifact.arch}`);
    const assetName = artifact.artifactUrl.split("/").pop();
    assert(existsSync(join(mergedOut, assetName)), `聚合发布包未落盘：${assetName}`);
  }

  runExpectFailure(
    process.execPath,
    [
      RELEASE_GEN,
      "--merge",
      "--version",
      manifest.version,
      "--out",
      relative(ROOT, join(platformRoot, "missing-platform")),
      "--artifact",
      relative(ROOT, sources[0]),
      "--require-platform",
      "linux/aarch64",
    ],
    "缺少必需平台产物：linux/aarch64",
  );
  return merged;
}

async function verifyFrontendNormalization() {
  // 生产模块是 TS；用 esbuild 转成 ESM 后直接调用同一份实现，
  // 避免在本脚本里复制第二份发布源判定逻辑。
  const esbuild = await import("esbuild");
  const source = readFileSync(SOURCE_SECURITY, "utf8");
  const compiled = await esbuild.transform(source, { loader: "ts", format: "esm", target: "es2020" });
  const temp = join(ROOT, ".tmp", "plugin-release-source-security.mjs");
  mkdirSync(dirname(temp), { recursive: true });
  writeFileSync(temp, compiled.code);
  const module = await import(`${pathToFileURL(temp).href}?t=${Date.now()}`);
  const raw = "https://cdn.example.com/axagent/wiki/release.json";
  const normalized = module.normalizePluginSource(raw);
  assert(normalized === `axagent-release+${raw}`, `发布地址归一化错误：${normalized}`);
  assert(module.isReleasePluginSource(normalized), "归一化后的发布源未被识别");
  assert(!module.isRemotePluginSource(normalized), "发布源不应按通用远程源要求手填哈希");
  assert(module.isRemotePluginSource("https://example.com/wiki.git"), "Git URL 未被识别为远程源");
  assert(module.isRemotePluginSource("@scope/name"), "scoped npm 包未被识别为远程源");
  assert(!module.isRemotePluginSource("C:\\plugins\\wiki"), "Windows 绝对路径被误判为远程源");
}

function verifyRustContract() {
  const manager = readFileSync(MANAGER_RS, "utf8");
  assert(manager.includes('const RELEASE_SOURCE_PREFIX: &str = "axagent-release+";'), "Rust 发布源前缀不一致");
  assert(manager.includes("PluginInstallSource::ReleaseManifestUrl"), "Rust 未登记 ReleaseManifestUrl 安装源");
  assert(manager.includes("pub fn validate_plugin_source_with_expected_hash"), "缺少预览哈希冻结方法");
  assert(manager.includes("pub fn install_verified"), "缺少 install_verified 安装门禁");
  assert(manager.includes("pub fn update_verified"), "缺少 update_verified 更新门禁");
  assert(/fn validate_expected_sha256\([\s\S]*?64/.test(manager), "SHA-256 64 位格式门禁缺失");

  const command = readFileSync(PLUGIN_CMD_RS, "utf8");
  assert(command.includes("validate_plugin_source_with_expected_hash"), "plugin_validate_source 未冻结预览哈希");
  assert(command.includes("install_verified"), "plugin_install 未调用 install_verified");
  assert(command.includes("update_verified"), "plugin_update 未调用 update_verified");
  assert(command.includes("PLUGIN_SOURCE_INTEGRITY_FAILED"), "安装错误未映射发布完整性错误码");
}

function verifyFrontendFreezeContract() {
  const gallery = readFileSync(GALLERY_TSX, "utf8");
  assert(
    /setConfirmSha256\(manifest\.expectedSha256\s*\?\?\s*expectedSha256\)/.test(gallery),
    "预览返回的 expectedSha256 未冻结进确认态",
  );
  assert(
    gallery.includes("installPlugin(confirmSource, confirmSha256 || undefined)"),
    "安装确认未把冻结哈希传给 plugin_install",
  );
  assert(
    gallery.includes("updatePlugin(updatingPluginId, updateConfirmSha256 || undefined)"),
    "更新确认未把冻结哈希传给 plugin_update",
  );
}

function verifyBundleReleaseShape(script = BUNDLE_SCRIPT) {
  const source = readFileSync(script, "utf8");
  for (const required of ["plugin.json", "pages/index.html"]) {
    assert(source.includes(required), `打包器未声明发布地址安装所需资产：${required}`);
  }
}

function stageIndependentBundles() {
  for (const plugin of PLUGIN_NAMES) {
    const repo = pluginRepo(ROOT, plugin);
    const result = spawnSync(
      process.execPath,
      [pluginBundleScript(ROOT, plugin), "--plugin", plugin, "--check"],
      { cwd: repo, stdio: "inherit", windowsHide: true, env: process.env },
    );
    if (result.error || result.status !== 0) {
      throw new Error(`独立仓 ${plugin} bundle 自检失败：${result.error?.message ?? `exit ${result.status}`}`);
    }
  }
}

async function main() {
  // CI 干净 checkout 没有 sibling 的 17 个独立仓，也没有它们的 dist/plugin.json ⇒
  // 验仓与发布物重建只能在本地集成入口跑（各独立仓自己的 CI/Release 也覆盖）。
  // 这里的 CI 安全面仍对主仓内的发布形状、发布源归一化、前后端冻结与 Rust 安装门禁
  // 逐项断言；它是子集，但**不是** `|| true` 静默放行，输出也明确标注范围。
  if (process.argv.includes("--ci-safe")) {
    run(process.execPath, [RELEASE_GEN, "--selftest"]);
    verifyBundleReleaseShape(MAIN_BUNDLE_SCRIPT);
    await verifyFrontendNormalization();
    verifyFrontendFreezeContract();
    verifyRustContract();
    console.log(JSON.stringify({
      scope: "main-repo-only contract surface; sibling repos and bundle artifacts excluded",
      releaseGeneratorSelftest: "PASS",
      bundleReleaseShape: "PASS",
      sourceNormalization: "PASS",
      frontendFreezeContract: "PASS",
      rustInstallContract: "PASS",
    }, null, 2));
    console.log("PLUGIN_RELEASE_FLOW_CI_SAFE_OK");
    return;
  }
  const output = run(process.execPath, [REPO_VERIFY, "--verify"]);
  assert(output.includes("PLUGIN_REPOS_CHECK_OK"), "独立仓快照自检未通过");
  run(process.execPath, [RELEASE_GEN, "--selftest"]);
  verifyBundleReleaseShape();
  await verifyFrontendNormalization();
  verifyFrontendFreezeContract();
  verifyRustContract();
  stageIndependentBundles();

  const pluginNames = [...PLUGIN_NAMES];
  assert(
    pluginNames.length === PLUGIN_CONTRACT.plugins.length && pluginNames.length > 0,
    `发布形态 bundle 数应与契约一致（${PLUGIN_CONTRACT.plugins.length}），实得 ${pluginNames.length}`,
  );

  const releases = [];
  let legacyRelease = null;
  for (const plugin of pluginNames) {
    const bundle = pluginBundleDist(ROOT, plugin);
    const out = join(ROOT, ".tmp", "plugin-release-flow", plugin);
    rmSync(out, { recursive: true, force: true });
    const manifest = JSON.parse(readFileSync(join(bundle, "plugin.json"), "utf8"));
    const artifactUrl = `https://cdn.example.com/axagent/${plugin}-${manifest.version}.tar.gz`;
    generatePlatformRelease(bundle, out, artifactUrl, "linux", "x86_64");
    releases.push(verifyReleaseArtifact(bundle, out, artifactUrl, "linux", "x86_64"));
    if (!legacyRelease) {
      const legacyOut = join(ROOT, ".tmp", "plugin-release-flow", "legacy", plugin);
      const legacyUrl = `https://cdn.example.com/axagent/${plugin}-${manifest.version}-legacy.tar.gz`;
      generatePlatformRelease(bundle, legacyOut, legacyUrl, "linux", "x86_64", true);
      legacyRelease = verifyReleaseArtifact(bundle, legacyOut, legacyUrl, "linux", "x86_64", true);
      verifyPlatformMerge(plugin, bundle, manifest);
    }
  }
  console.log(JSON.stringify({
    independentRepos: "17/17",
    releaseSource: "axagent-release+https://.../release.json",
    releaseBundles: `${releases.length}/${releases.length}`,
    platformMerge: "6/6",
    missingPlatformGuard: "PASS",
    legacyManifest: "PASS",
    sample: {
      artifactUrl: releases[0].artifact.artifactUrl,
      sha256: releases[0].artifact.sha256,
      version: releases[0].version,
    },
    legacySample: legacyRelease ? {
      artifactUrl: legacyRelease.artifact.artifactUrl,
      sha256: legacyRelease.artifact.sha256,
      version: legacyRelease.version,
    } : null,
    scope: "contract regression only; no GUI or cargo publish claim",
  }, null, 2));
  console.log("PLUGIN_RELEASE_FLOW_OK");
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
});
