#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 一次性引导工具：从主仓历史源码生成“每插件一个独立 Git 仓库”快照。
 *
 * 正式源码权威位置已经是主仓外的 17 个独立 Git 仓。常规 CI 与开发只应使用
 * `--verify`，它校验现有仓的依赖边界、bundle、发布工作流和稳定 release.json 地址；
 * 只有首次建仓或灾后重建时才显式传入 `--bootstrap-from-main` 读取主仓旧源码。
 *
 * 每个插件仓都自带最小 SDK 源码快照，并只通过仓内 path 依赖构建。
 * 这样发布工作流不依赖主仓源码，也不把 17 个仓的共同成败绑到尚未发布的
 * crates.io SDK 上；生成器负责保证各仓 SDK 副本与主仓同源。
 *
 * 用法：
 *   node scripts/generate-plugin-independent-repos.mjs --verify
 *   node scripts/generate-plugin-independent-repos.mjs --bootstrap-from-main --all
 *   node scripts/generate-plugin-independent-repos.mjs --bootstrap-from-main --all --skip-git --out /tmp/repos
 */
import { spawnSync } from "node:child_process";
import {
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";
import { PLUGIN_NAMES, pluginRepo, pluginReposRoot } from "./lib/plugin-repos.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const DEFAULT_OUT = pluginReposRoot(ROOT);
const argv = process.argv.slice(2);
const RELEASE_OWNER = flag("--owner") ?? "polite0803";
const RELEASE_REPOS_FILE = ".axinvest-plugin-repos.json";
// 唯一契约源：版本、形态、worker crate 全部从这里派生，不再手抄第二份。
// Release workflow 按 ref:${{ env.RELEASE_TAG }} checkout，版本号漂移会让
// workflow_dispatch 传入的旧 tag 与 plugin.json 版本断言失败。
const CONTRACT = JSON.parse(readFileSync(join(ROOT, "scripts", "lib", "plugin-contract.json"), "utf8"));
const RUST_RELEASE_VERSION = CONTRACT.releases.workerVersion;
const RUST_TOOLCHAIN = "1.97.0";
const DECLARATIVE_RELEASE_VERSION = CONTRACT.releases.legacyDeclarativeVersion;
const RELEASE_PLATFORMS = [
  { os: "linux", arch: "x86_64" },
  { os: "linux", arch: "aarch64" },
  { os: "macos", arch: "x86_64" },
  { os: "macos", arch: "aarch64" },
  { os: "windows", arch: "x86_64" },
  { os: "windows", arch: "aarch64" },
];

/** 正式插件仓名：插件名 → GitHub repository 名。 */
function releaseRepoName(plugin) {
  return `axinvest-plugin-${plugin}`;
}

/** 主应用可直接输入、随 latest Release 稳定更新的清单地址。 */
function releaseManifestUrl(plugin) {
  return releaseManifestUrlForOwner(plugin, RELEASE_OWNER);
}

function releaseManifestUrlForOwner(plugin, owner) {
  return `https://github.com/${owner}/${releaseRepoName(plugin)}/releases/latest/download/release.json`;
}

function readReleaseReposFile(repo) {
  const path = join(repo, RELEASE_REPOS_FILE);
  if (!existsSync(path)) return null;
  try {
    const value = JSON.parse(readFileSync(path, "utf8"));
    if (typeof value.owner !== "string" || !value.owner.trim()) {
      fail(`${RELEASE_REPOS_FILE} 缺少合法 owner`);
    }
    if (typeof value.plugin !== "string" || !value.plugin.trim()) {
      fail(`${RELEASE_REPOS_FILE} 缺少合法 plugin`);
    }
    return value;
  } catch (error) {
    fail(`${RELEASE_REPOS_FILE} 不是合法 JSON：${error instanceof Error ? error.message : String(error)}`);
  }
}

function flag(name) {
  const index = argv.indexOf(name);
  return index >= 0 ? argv[index + 1] : null;
}

function hasFlag(name) {
  return argv.includes(name);
}

function shell(command, args, cwd) {
  const result = spawnSync(command, args, { cwd, encoding: "utf8", windowsHide: true });
  return { ok: !result.error && result.status === 0, output: `${result.stdout ?? ""}${result.stderr ?? ""}`.trim() };
}

function verifyGeneratedRepos() {
  if (!existsSync(OUT_ROOT)) fail(`输出根不存在：${OUT_ROOT}`);
  const expected = Object.keys(SPECS);
  if (expected.length !== PLUGIN_NAMES.length) {
    fail(`生成器规格 ${expected.length} 与插件仓清单 ${PLUGIN_NAMES.length} 不一致`);
  }
  const repos = readdirSync(OUT_ROOT)
    .map((name) => join(OUT_ROOT, name))
    .filter((dir) => statSync(dir).isDirectory() && existsSync(join(dir, ".git")))
    .sort();
  const unexpected = repos
    .map((repo) => repo.split(sep).pop())
    .filter((name) => !expected.includes(name));
  if (unexpected.length) {
    fail(`输出根含非插件 Git 仓：${unexpected.join("、")}`);
  }
  // 域包底册由本生成器 copyTree 到每个独立仓。验收矩阵读的是 opc 独立仓副本，
  // 任何一仓滞后都会让它按旧 owner / 旧 capabilities 把已迁移的 worker 域误判成
  // 声明式（阶段 1 实测：game_dev 迁移后矩阵仍报 owner=plugin-opc-common）。
  // 这是本地集成门的逐项对账，不放主仓 check-plugin-contract.mjs（后者刻意不读仓外目录）。
  const registrySource = readFileSync(
    join(ROOT, "axagent-plugins", "scripts", "opc-domain-pack-plugins.json"),
    "utf8",
  );
  const staleRegistry = repos
    .filter((repo) => {
      const path = join(repo, "scripts", "opc-domain-pack-plugins.json");
      return !existsSync(path) || readFileSync(path, "utf8") !== registrySource;
    })
    .map((repo) => repo.split(sep).pop());
  if (staleRegistry.length) {
    fail(`以下独立仓的 scripts/opc-domain-pack-plugins.json 与主仓不一致：${staleRegistry.join("、")}`);
  }
  const requiredFiles = [
    ".gitattributes",
    ".github/workflows/plugin-ci.yml",
    ".github/workflows/release-plugin.yml",
    "scripts/check-release-dependencies.mjs",
    "scripts/build-plugin-bundle.mjs",
    "scripts/opc-domain-pack-plugins.json",
    "generate-plugin-release-manifest.mjs",
    "rust-toolchain.toml",
  ];
  const results = repos.map((repo) => {
    const plugin = repo.split(sep).pop();
    const releaseRepos = readReleaseReposFile(repo);
    if (releaseRepos && releaseRepos.plugin !== plugin) {
      fail(`${plugin} 仓的 ${RELEASE_REPOS_FILE} plugin 字段应为 ${plugin}，实得 ${releaseRepos.plugin}`);
    }
    const repoOwner = releaseRepos?.owner?.trim() || RELEASE_OWNER;
    const repoName = releaseRepos?.repo?.trim() || releaseRepoName(plugin);
    const expectedPluginVersion = releaseRepos?.pluginVersion;
    if (!expectedPluginVersion || !/^\d+\.\d+\.\d+/.test(expectedPluginVersion)) {
      fail(`${plugin} 仓的 ${RELEASE_REPOS_FILE} 缺少合法 pluginVersion`);
    }
    const script = join(repo, "scripts", "build-plugin-bundle.mjs");
    const missingFiles = requiredFiles.filter((file) => !existsSync(join(repo, file)));
    const workflowViolations = requiredFiles
      .filter((file) => file.startsWith(".github/workflows/"))
      .map((file) => ({ file, text: existsSync(join(repo, file)) ? readFileSync(join(repo, file), "utf8") : "" }))
      .flatMap(({ file, text }) => {
        const violations = [];
        if (!text) {
          violations.push(`${file} 缺失`);
        } else {
          if (!text.includes("node scripts/build-plugin-bundle.mjs")) {
            violations.push(`${file} 未执行 bundle 构建`);
          }
          if (!text.includes("node scripts/check-release-dependencies.mjs")) {
            violations.push(`${file} 未执行独立仓依赖检查`);
          }
          if (/\|\|\s*true|continue-on-error\s*:\s*true/.test(text)) {
            violations.push(`${file} 存在静默放行`);
          }
        }
        return violations;
      });
    const releaseText = existsSync(join(repo, ".github/workflows/release-plugin.yml"))
      ? readFileSync(join(repo, ".github/workflows/release-plugin.yml"), "utf8")
      : "";
    if (
      releaseText
      && (!releaseText.includes("gh release create")
        || !releaseText.includes("generate-plugin-release-manifest.mjs")
        || !releaseText.includes("release.json")
        || !releaseText.includes("--merge")
        || !releaseText.includes("--stage-artifacts"))
    ) {
      workflowViolations.push(".github/workflows/release-plugin.yml 未形成 release.json/tar.gz GitHub Release");
    }
    const readmePath = join(repo, "README.md");
    const readme = existsSync(readmePath) ? readFileSync(readmePath, "utf8") : "";
    const expectedManifestUrl = `https://github.com/${repoOwner}/${repoName}/releases/latest/download/release.json`;
    if (!readme.includes(expectedManifestUrl)) {
      workflowViolations.push(`README.md 未提供主应用可用的稳定发布地址：${expectedManifestUrl}`);
    }
    // vendored 脚本必须排除出 GitHub Linguist 统计：build-plugin-bundle.mjs 单文件约 270 KB，
    // 不排除时会把纯声明式插件仓的语言标签压成 99% JavaScript（实测 opc-finance-invest）。
    const attrsPath = join(repo, ".gitattributes");
    const attrsText = existsSync(attrsPath) ? readFileSync(attrsPath, "utf8") : "";
    for (const vendored of [
      "scripts/build-plugin-bundle.mjs",
      "scripts/check-release-dependencies.mjs",
      "generate-plugin-release-manifest.mjs",
    ]) {
      if (!new RegExp(`^${vendored.replace(/[.*+?^${}()|[\\]\\]/g, "\\$&")}\\s+linguist-vendored\\s*$`, "m").test(attrsText)) {
        workflowViolations.push(`.gitattributes 未将 ${vendored} 标为 linguist-vendored`);
      }
    }
    if (attrsText.includes("\r\n")) {
      workflowViolations.push(".gitattributes 必须使用 LF 行尾");
    }
    const declarative = !WORKER_SPECS[plugin];
    if (declarative && existsSync(join(repo, "Cargo.toml"))) {
      workflowViolations.push("声明式插件仓不得包含 Cargo.toml");
    }
    const rootManifestText = existsSync(join(repo, "Cargo.toml"))
      ? readFileSync(join(repo, "Cargo.toml"), "utf8")
      : "";
    const sdkManifestPath = join(repo, "sdk", "Cargo.toml");
    if (!declarative && rootManifestText.includes("axagent-plugin-") && !existsSync(sdkManifestPath)) {
      workflowViolations.push("worker 仓缺少自包含 SDK 工作区 sdk/Cargo.toml");
    }
    if (/axagent-(?:plugin-(?:proto|opc-core|testkit)|harness)\s*=\s*"3\.0\.0"/.test(rootManifestText)) {
      workflowViolations.push("worker 仓 SDK 仍是外部 registry 版本依赖");
    }
    if (releaseText) {
      for (const platform of RELEASE_PLATFORMS) {
        if (!releaseText.includes(`${platform.os}/${platform.arch}`)) {
          workflowViolations.push(
            `.github/workflows/release-plugin.yml 缺少平台 ${platform.os}/${platform.arch}`,
          );
        }
      }
      for (const required of ["--release", "--target", "actions/upload-artifact@", "actions/download-artifact@"]) {
        if (!releaseText.includes(required)) {
          workflowViolations.push(`.github/workflows/release-plugin.yml 缺少 ${required}`);
        }
      }
      for (const required of ["--os", "--arch"]) {
        if (!releaseText.includes(required)) {
          workflowViolations.push(`.github/workflows/release-plugin.yml 缺少平台清单参数 ${required}`);
        }
      }
    }
    // 防复发：以下四类都是 GitHub 上真实踩过的失败形态（详见 PLAN 的 CI-A/B/C、REL-A/B），
    // 只查文件存在与命令关键字不足以发现它们，必须对生成物做内容形态断言。
    const ciText = existsSync(join(repo, ".github/workflows/plugin-ci.yml"))
      ? readFileSync(join(repo, ".github/workflows/plugin-ci.yml"), "utf8")
      : "";
    for (const [file, text] of [
      [".github/workflows/plugin-ci.yml", ciText],
      [".github/workflows/release-plugin.yml", releaseText],
    ]) {
      if (text.includes("if [ -f Cargo.toml ]")) {
        workflowViolations.push(`${file} 仍用 bash if 判断 Cargo.toml（Windows PowerShell 会 ParserError）`);
      }
    }
    const hasRustWorker = Boolean(WORKER_SPECS[plugin]);
    for (const [file, text] of [
      [".github/workflows/plugin-ci.yml", ciText],
      [".github/workflows/release-plugin.yml", releaseText],
    ]) {
      const hasCargoStep = text.includes("cargo fmt --all --check");
      if (hasRustWorker !== hasCargoStep) {
        workflowViolations.push(
          `${file} ${hasRustWorker ? "缺少" : "不应包含"}无条件 cargo fmt --all --check 步骤`,
        );
      }
    }
    const harnessSource = join(repo, "sdk", "harness", "src", "knowledge_graph.rs");
    if (existsSync(harnessSource)
      && readFileSync(harnessSource, "utf8").includes('include_str!("../../../../src-tauri/')) {
      workflowViolations.push("SDK harness 快照仍含跨仓 include_str!(../../../../src-tauri/...)");
    }
    const bundleText = existsSync(script) ? readFileSync(script, "utf8") : "";
    if (plugin === "opc"
      && (!bundleText.includes('fs.existsSync(path.join(workspaceRoot, "plugin-opc", "domain_packs"))')
        || !bundleText.includes('const domainPackManifests = (fs.existsSync'))) {
      workflowViolations.push("opc bundle 脚本的 domain_packs 读取缺少 existsSync 容错（CI checkout 无该目录）");
    }
    const vendoredManifest = join(repo, "generate-plugin-release-manifest.mjs");
    if (existsSync(vendoredManifest)
      && !readFileSync(vendoredManifest, "utf8").includes('basename(SCRIPT_DIR) === "scripts"')) {
      workflowViolations.push("vendored generate-plugin-release-manifest.mjs 的 ROOT 仍按主仓 scripts/ 布局解析");
    }
    // `--check` 会把 worker 和 manifest 写进 dist/。多个验仓进程共用同一仓时必须隔离输出，
    // 否则一个进程的 manifest 可能被另一个进程的二进制覆盖，产生与源码无关的哈希假红。
    const checkOut = mkdtempSync(join(tmpdir(), `axinvest-plugin-check-${plugin}-`));
    let check;
    let dependencyCheck;
    let releaseGeneratorCheck;
    try {
      dependencyCheck = existsSync(join(repo, "scripts/check-release-dependencies.mjs"))
        ? shell(process.execPath, [join(repo, "scripts/check-release-dependencies.mjs")], repo)
        : { ok: false, output: "缺少 scripts/check-release-dependencies.mjs" };
      releaseGeneratorCheck = existsSync(join(repo, "generate-plugin-release-manifest.mjs"))
        ? shell(process.execPath, [join(repo, "generate-plugin-release-manifest.mjs"), "--selftest"], repo)
        : { ok: false, output: "缺少 generate-plugin-release-manifest.mjs" };
      check = existsSync(script)
        ? shell(
          process.execPath,
          [
            script,
            "--plugin",
            plugin,
            "--plugin-version",
            expectedPluginVersion,
            "--check",
            "--out",
            checkOut,
          ],
          repo,
        )
        : { ok: false, output: "缺少 scripts/build-plugin-bundle.mjs" };
      const builtManifestPath = join(checkOut, "plugin.json");
      const builtPluginVersion = check.ok && existsSync(builtManifestPath)
        ? JSON.parse(readFileSync(builtManifestPath, "utf8")).version
        : null;
      if (builtPluginVersion !== expectedPluginVersion) {
        workflowViolations.push(
          `plugin.json 版本 ${builtPluginVersion ?? "(missing)"} 与发布元数据 ${expectedPluginVersion} 不一致`,
        );
      }
    } finally {
      rmSync(checkOut, { recursive: true, force: true });
    }
    return {
      plugin,
      git: existsSync(join(repo, ".git")),
      missingFiles,
      workflowViolations,
      dependencyCheck: dependencyCheck.ok,
      dependencyDetail: dependencyCheck.output.split("\n").pop() ?? "",
      releaseGeneratorCheck: releaseGeneratorCheck.ok,
      releaseGeneratorDetail: releaseGeneratorCheck.output.split("\n").pop() ?? "",
      bundleCheck: check.ok
        && dependencyCheck.ok
        && releaseGeneratorCheck.ok
        && missingFiles.length === 0
        && workflowViolations.length === 0,
      detail: check.output.split("\n").pop() ?? "",
    };
  });
  const missing = expected.filter((name) => !results.some((row) => row.plugin === name));
  const failed = results.filter((row) => !row.bundleCheck).map((row) => row.plugin);
  console.log(JSON.stringify({ outputRoot: OUT_ROOT, expected: expected.length, repos: results, missing, failed }, null, 2));
  if (missing.length || failed.length) {
    console.error(`PLUGIN_REPOS_CHECK_FAIL: 缺失 ${missing.length} / bundle 失败 ${failed.length}`);
    process.exit(1);
  }
  console.log("PLUGIN_REPOS_CHECK_OK");
}

async function verifyReleaseReposMetadata() {
  const { verifyPluginReleaseRepos } = await import("./publish-plugin-independent-repos.mjs");
  const results = verifyPluginReleaseRepos({ root: OUT_ROOT, owner: RELEASE_OWNER });
  console.log(`PLUGIN_RELEASE_REPOS_METADATA_OK ${results.length}/17`);
}

function fail(message) {
  console.error(`[plugin-repos] FAIL: ${message}`);
  process.exit(1);
}

function run(command, args, cwd, label) {
  const result = spawnSync(command, args, { cwd, stdio: "inherit", windowsHide: true });
  if (result.error || result.status !== 0) {
    fail(`${label} 失败：${result.error?.message ?? `exit ${result.status}`}`);
  }
}

function commitIfChanged(repo, message, label) {
  if (!shell("git", ["status", "--porcelain", RELEASE_REPOS_FILE], repo).output) return;
  run("git", ["add", RELEASE_REPOS_FILE], repo, `${label} git add`);
  run(
    "git",
    ["-c", "user.name=Codex", "-c", "user.email=codex@example.com", "commit", "-qm", message],
    repo,
    label,
  );
}

function read(rel) {
  return readFileSync(join(ROOT, rel), "utf8");
}

function requestedVersion() {
  return RUST_RELEASE_VERSION;
}

function parseBundleSpecs() {
  const src = read("axagent-plugins/scripts/build-plugin-bundle.mjs");
  const start = src.indexOf("const PLUGIN_SPECS = {");
  const end = src.indexOf("\n};", start);
  if (start < 0 || end <= start) fail("无法从打包器读取 PLUGIN_SPECS");
  const block = src.slice(start, end);
  const specs = {};
  const entries = [...block.matchAll(/^  "?([a-z0-9-]+)"?:\s*\{/gm)];
  for (let i = 0; i < entries.length; i += 1) {
    const match = entries[i];
    const name = match[1];
    const specBody = block.slice(match.index, entries[i + 1]?.index ?? block.length);
    specs[name] = {
      packageName: specBody.match(/packageName:\s*"([^"]+)"/)?.[1] ?? null,
      webDir: specBody.match(/webDir:\s*path\.join\(([^)]+)\)/)?.[1] ?? null,
    };
  }
  return specs;
}

// 打包器规格现在只作对账辅助：生成产物形状全部由契约决定。
// 二者的一致性由 `node scripts/check-plugin-contract.mjs` 在 CI 两侧逐项校验。
const SPECS = parseBundleSpecs();
const CONTRACT_BY_NAME = new Map(CONTRACT.plugins.map((p) => [p.name, p]));
const WORKER_SPECS = Object.fromEntries(
  CONTRACT.plugins
    .filter((p) => p.strategy === "worker")
    .map((p) => [
      p.name,
      { crate: p.crate, packageName: p.packageName, sdk: p.sdk, extraCrates: p.extraCrates },
    ]),
);

const SYNC_EXISTING = hasFlag("--sync-existing");
const DRY_RUN = hasFlag("--dry-run");

function selectedPlugins() {
  const one = flag("--plugin");
  if (SYNC_EXISTING) {
    // 同步模式一次只动一个真实仓；批量同步会让「只覆盖生成文件」的边界难以逐仓审查。
    if (hasFlag("--all") || one === "all" || !one) fail("--sync-existing 必须且只能配 --plugin <名称>");
    if (!CONTRACT_BY_NAME.has(one)) fail(`未知插件 ${one}（可选：${[...CONTRACT_BY_NAME.keys()].join(" | ")}）`);
    return [one];
  }
  if (hasFlag("--all") || one === "all") return Object.keys(SPECS);
  if (!one) fail("需要 --plugin <名称> 或 --all");
  if (!SPECS[one]) fail(`未知插件 ${one}（可选：${Object.keys(SPECS).join(" | ")}）`);
  return [one];
}

let OUT_ROOT = resolve(flag("--out") ?? DEFAULT_OUT);
let SKIP_GIT = argv.includes("--skip-git");
// 同步模式绝不重建 Git 历史：默认模式 :rmSync(repo) 会连 .git/ 一起删。
if (SYNC_EXISTING && !SKIP_GIT) {
  fail("--sync-existing 必须配 --skip-git（本脚本不做 Git 提交，只同步工作树文件）");
}

// 同步时保留的目录：Git 元数据、构建产物、依赖都不由生成器拥有。
const SYNC_PROTECTED = new Set([".git", "dist", "target", "node_modules"]);

if (hasFlag("--verify")) {
  verifyGeneratedRepos();
  await verifyReleaseReposMetadata();
  process.exit(0);
}

if (!hasFlag("--bootstrap-from-main") && !SYNC_EXISTING) {
  fail("生成模式需要显式 --bootstrap-from-main；常规验收请用 --verify");
}

const plugins = selectedPlugins();

function wsDeps(block, extra = "") {
  const shared = [
    'anyhow = "1"',
    'async-trait = "0.1"',
    'chrono = { version = "0.4", features = ["serde"] }',
    'futures = "0.3"',
    'hex = "0.4"',
    'parking_lot = "0.12"',
    'regex = "1"',
    'reqwest = { version = "0.13", features = ["json", "cookies"] }',
    'rhai = { version = "1", features = ["sync"] }',
    'schemars = "1.2"',
    'sea-orm = "2"',
    'serde = { version = "1", features = ["derive"] }',
    'serde_json = "1"',
    'serde_yaml = "0.9"',
    'sha2 = "0.11"',
    'static_assertions = "1"',
    'thiserror = "2"',
    'tokio = { version = "1", features = ["full"] }',
    'tracing = "0.1"',
    'ts-rs = { version = "12", features = ["serde-json-impl", "no-serde-warnings"] }',
    'uuid = { version = "1", features = ["v4", "serde"] }',
  ];
  return `[workspace.dependencies]\n${[...shared, ...extra.split("\n").filter(Boolean)].join("\n")}\n\n`;
}

function rootManifest(name, members, sdkDirs, version = RELEASE_VERSION) {
  const sdkWorkspaceDeps = sdkDirs
    ? `axagent-plugin-proto = { version = "${version}", path = "sdk/plugin-proto" }\n` +
      `axagent-plugin-opc-core = { version = "${version}", path = "sdk/plugin-opc-core" }\n` +
      `axagent-plugin-testkit = { version = "${version}", path = "sdk/plugin-testkit" }\n` +
      `axagent-harness = { version = "${version}", path = "sdk/harness" }`
    : "";
  const deps = wsDeps(
    "",
    sdkWorkspaceDeps,
  );
  return `# ${name} 独立插件仓（自包含 SDK 快照）\n` +
    `[workspace]\nresolver = "2"\nmembers = [\n${members.map((m) => `  "${m}",`).join("\n")}\n]\n\n` +
    `[workspace.package]\nversion = "${version}"\nedition = "2024"\nlicense = "AGPL-3.0-only"\n\n` +
    deps +
    `[profile.dev]\nopt-level = 0\ndebug = "line-tables-only"\nincremental = false\nsplit-debuginfo = "unpacked"\n\n` +
    `[profile.dev.build-override]\nopt-level = 0\ncodegen-units = 16\ndebug = false\nincremental = false\n`;
}

function rewriteManifest(text, sdkDirs, rewriteWorkspaceDeps = false, version = RELEASE_VERSION) {
  let out = text;
  out = out.replace(/version\.workspace = true/g, `version = "${version}"`);
  out = out.replace(/edition\.workspace = true/g, 'edition = "2024"');
  out = out.replace(/license\.workspace = true/g, 'license = "AGPL-3.0-only"');
  out = out.replace(/\[lints\]\nworkspace = true\n?/g, "");
  out = out.replace(/^version\s*=\s*"\d+\.\d+\.\d+[^"]*"/m, `version = "${version}"`);
  if (rewriteWorkspaceDeps) {
    // harness 的 axagent-* 依赖要脱落 workspace；第三方依赖保留原版本，只在
    // workspace 根提供版本。这里只改写 axagent-* 键，避免把 anyhow 写成 3.x。
    out = out.replace(
      /^(\s*axagent-[a-zA-Z0-9_-]+)\s*=\s*\{\s*workspace = true\s*,\s*features = \[([^\]]*)\]\s*\}$/gm,
      `$1 = { version = "${version}", features = [$2] }`,
    );
    out = out.replace(
      /^(\s*axagent-[a-zA-Z0-9_-]+)\s*=\s*\{\s*workspace = true\s*\}$/gm,
      `$1 = "${version}"`,
    );
    // 真正的 workspace = true（第三方公共版本）改从 workspace 继承。
    out = out.replace(
      /^(\s*[a-zA-Z0-9_-]+)\s*=\s*\{\s*workspace = true\s*,\s*features = \[([^\]]*)\]\s*\}$/gm,
      '$1 = { workspace = true, features = [$2] }',
    );
  }
  if (sdkDirs) {
    out = out.replace(
      /axagent-plugin-proto\s*=\s*\{[^}]*\}/g,
      `axagent-plugin-proto = { version = "${version}", path = "../sdk/plugin-proto" }`,
    );
    out = out.replace(
      /axagent-plugin-opc-core\s*=\s*\{[^}]*\}/g,
      `axagent-plugin-opc-core = { version = "${version}", path = "../sdk/plugin-opc-core" }`,
    );
    out = out.replace(
      /axagent-plugin-testkit\s*=\s*\{[^}]*\}/g,
      `axagent-plugin-testkit = { version = "${version}", path = "../sdk/plugin-testkit" }`,
    );
    out = out.replace(
      /axagent-harness\s*=\s*\{[^}]*\}/g,
      `axagent-harness = { version = "${version}", path = "../sdk/harness" }`,
    );
  }
  return out;
}

function copyTree(srcRel, dstRel) {
  const src = join(ROOT, srcRel);
  if (!existsSync(src)) fail(`源不存在：${srcRel}`);
  const dst = dstRel;
  mkdirSync(dirname(dst), { recursive: true });
  cpSync(src, dst, { recursive: true, force: true, verbatimSymlinks: true });
}

function copySdks(repo, sdkNames, version) {
  const sdkDirs = new Set(["plugin-proto", ...sdkNames]);
  mkdirSync(join(repo, "sdk"), { recursive: true });
  for (const sdk of sdkDirs) {
    const srcRel = sdk === "plugin-proto"
      ? "axagent-core/crates/plugin-proto"
      : sdk === "harness"
      ? "axagent-core/crates/harness"
      : `axagent-plugins/crates/${sdk}`;
    const dst = join(repo, "sdk", sdk);
    copyTree(srcRel, dst);
    if (sdk === "harness") {
      // 主仓 harness 有一个 #[cfg(test)] 用例通过 include_str! 回溯到单仓时代的
      // src-tauri/src/commands/knowledge.rs。主仓布局下该路径有效，但独立仓只有
      // sdk/ 快照，cargo check --all-targets 会因找不到文件而编译失败（stock CI 实测）。
      // 只剥离这一个测试（含其前置文档注释），不碰任何生产代码；其余测试照常保留。
      const sourcePath = join(dst, "src", "knowledge_graph.rs");
      const source = readFileSync(sourcePath, "utf8");
      const marker = "fn test_csv_import_relations_are_declared()";
      const fnIndex = source.indexOf(marker);
      if (fnIndex < 0) fail("SDK harness 快照缺少 test_csv_import_relations_are_declared，无法定点剥离");
      const nextTestIndex = source.indexOf("    #[test]", fnIndex);
      if (nextTestIndex < 0) fail("SDK harness 快照测试边界定位失败");
      // 往前找到紧邻该 #[test] 的注释块起点（跳过注释、空行及上一测试收尾的空行），
      // 再将整段替换为下一个 #[test] 的起始，等价于删除这一整个测试。
      const before = source.slice(0, fnIndex);
      const testAttr = before.lastIndexOf("    #[test]");
      if (testAttr < 0) fail("SDK harness 快照测试属性定位失败");
      const preceding = source.slice(0, testAttr);
      const docMatch = /(?:^[ \t]*\/\/\/[^\n]*(?:\n|$)|^[ \t]*\/\/[^\n]*(?:\n|$)|^[ \t]*\n)+$/.exec(preceding);
      const docStart = docMatch ? testAttr - docMatch[0].length : testAttr;
      const stripped = source.slice(0, docStart) + source.slice(nextTestIndex);
      if (stripped.includes('include_str!("../../../../src-tauri/')) {
        fail("SDK harness 快照剥离后仍残留跨仓 include_str!");
      }
      writeFileSync(sourcePath, stripped);
    }
    const manifestPath = join(dst, "Cargo.toml");
    const rewritten = rewriteManifest(readFileSync(manifestPath, "utf8"), true, true, version)
      .replace(/path\s*=\s*"[^"]*plugin-proto"/g, 'path = "../plugin-proto"')
      .replace(/\[lints\.clippy\][\s\S]*$/m, "");
    writeFileSync(manifestPath, rewritten);
  }
  writeFileSync(join(repo, "sdk", "Cargo.toml"), rootManifest("sdk", [...sdkDirs].sort(), true, version));
  return sdkDirs;
}

function generateWorkerRepo(name, repo, version) {
  const spec = WORKER_SPECS[name];
  const crateDir = `plugin-${name}`;
  const extraCrates = spec.extraCrates ?? [];
  const members = [crateDir, ...extraCrates];
  copyTree(`axagent-plugins/crates/${spec.crate}`, join(repo, crateDir));
  // 打包器的 stock 规格引用 ui.json 校验任务源 detailSchema；独立仓必须带上同一份 schema。
  if (name === "stock") {
    copyTree("axagent-plugins/crates/plugin-stock/ui.json", join(repo, crateDir, "ui.json"));
  }
  // 页面验收脚本用 jsdom 在 Node 里装载 bundle；独立仓必须自带最小 package.json，
  // 否则从仓外执行 --check 会向上找不到主仓 node_modules。
  if (name === "stock") {
    writeFileSync(
      join(repo, "package.json"),
      JSON.stringify({ name: "plugin-stock-repo", private: true, devDependencies: { jsdom: "^30.0.1" } }, null, 2) + "\n",
    );
  }
  for (const extra of extraCrates) {
    const extraDir = join(repo, extra);
    copyTree(`axagent-plugins/crates/${extra}`, extraDir);
    const extraManifest = join(extraDir, "Cargo.toml");
    writeFileSync(
      extraManifest,
      rewriteManifest(readFileSync(extraManifest, "utf8"), true, false, version),
    );
  }
  const sdkDirs = copySdks(repo, spec.sdk, version);
  const manifestPath = join(repo, crateDir, "Cargo.toml");
  let rewritten = rewriteManifest(readFileSync(manifestPath, "utf8"), sdkDirs, false, version);
  if (extraCrates.includes("stock-core")) {
    rewritten = rewritten.replace(
      /axagent-stock-core\s*=\s*\{[^}]*\}/g,
      `axagent-stock-core = { version = "${version}", path = "../stock-core" }`,
    );
  }
  writeFileSync(manifestPath, rewritten);
  writeFileSync(join(repo, "Cargo.toml"), rootManifest(name, members, true, version));
  const nestedSdk = join(repo, "sdk", "sdk");
  if (existsSync(nestedSdk)) rmSync(nestedSdk, { recursive: true, force: true });
  return { packageName: spec.packageName, crate: crateDir, sdkDirs: [...sdkDirs] };
}

function generateDeclarativeRepo(name, repo, version) {
  copyTree(`axagent-plugins/crates/plugin-${name}/web`, join(repo, `plugin-${name}`, "web"));
  copyTree(`axagent-plugins/crates/plugin-${name}/domain_packs`, join(repo, `plugin-${name}`, "domain_packs"));
  return {
    packageName: null,
    crate: null,
    sdkDirs: [],
  };
}

function bundleScript(repo, name) {
  const bundle = read("axagent-plugins/scripts/build-plugin-bundle.mjs");
  const start = bundle.indexOf("const PLUGIN_SPECS = {");
  const end = bundle.indexOf("\n};", start);
  const block = bundle.slice(start, end);
  const lineStart = block.search(new RegExp(`^  "?${name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}"?:\\s*\\{`, "m"));
  if (lineStart < 0) fail(`打包器缺少 ${name} 规格`);
  const allKeys = [...block.matchAll(/^  "?([a-z0-9-]+)"?:\s*\{/gm)];
  const nextKey = allKeys.find((entry) => entry.index > lineStart)?.index ?? -1;
  const specBlock = block.slice(lineStart, nextKey < 0 ? block.length : nextKey);
  const declarative = !WORKER_SPECS[name];
  const body = declarative
    ? specBlock
        .replace(/webDir:\s*path\.join\([^)]*\)/, `webDir: path.join("plugin-${name}", "web")`)
        .replace(/uiSchema:\s*path\.join\([^)]*\)/, "uiSchema: null")
    : specBlock
        .replace(/webDir:\s*path\.join\([^)]*\)/, `webDir: path.join("plugin-${name}", "web")`)
        .replace(/uiSchema:\s*(path\.join\([^)]*\))/, (match, expression) => {
          const sourcePath = match.match(/path\.join\(([^)]*)\)/)?.[1] ?? "";
          const sourceFile = sourcePath.split(",").pop()?.trim().replace(/^"|"$/g, "") ?? "";
          const sourceCrate = sourcePath.includes("plugin-stock") ? "plugin-stock" : `plugin-${name}`;
          if (!sourceFile) return `uiSchema: ${expression}`;
          // 只重写当前插件 crate 内的 schema，避免把其它插件的 uiSchema 误指到本仓。
          if (sourcePath.includes("plugin-") && !sourceCrate.endsWith(name)) return `uiSchema: ${expression}`;
          return `uiSchema: path.join("${sourceCrate}", "${sourceFile}")`;
        });
  const narrowed = bundle.slice(0, start) + "const PLUGIN_SPECS = {\n" + body + "\n};\n" + bundle.slice(end + 3);
  // 独立仓只保留当前插件规格；其他插件的 manifest 构造器保留，避免手改大段函数体。
  const relocated = narrowed
    .replace(
      'const workspaceRoot = path.resolve(import.meta.dirname, "..");',
      'const workspaceRoot = path.resolve(import.meta.dirname, "..");',
    )
    .replace(
      'path.join(workspaceRoot, "crates", "plugin-opc", "domain_packs")',
      'path.join(workspaceRoot, "plugin-opc", "domain_packs")',
    )
    // opc 自身不拥有任何域包（14 份域包全部分给各域仓），该目录在 Git 中为空 ⇒
    // CI checkout 后不存在。无条件 readdirSync 会让 bundle 以 ENOENT 失败（远端实测）。
    // 改写整段 opcManifest 的目录读取：目录不存在时返回空列表（与真实内容等价）。
    .replace(
      /const domainPackManifests = fs\s*\.readdirSync\(path\.join\(workspaceRoot, "plugin-opc", "domain_packs"\)\)\s*\.filter/,
      'const domainPackManifests = (fs.existsSync(path.join(workspaceRoot, "plugin-opc", "domain_packs"))\n' +
      '      ? fs.readdirSync(path.join(workspaceRoot, "plugin-opc", "domain_packs"))\n' +
      '      : [])\n' +
      '    .filter',
    )
    .replace(
      /const domainPackSource = domainPackSourceByPlugin\[pluginName\] \?\? null;/,
      '// 独立仓没有 crates/ 布局：直接按仓根布局重建映射，不用字符串替换。\n' +
        '// 原 .replace(`/crates/...`) 只在 POSIX 上命中；Windows 的 path.join 产出反斜杠，\n' +
        '// 替换失败会让 Release 的 Windows job 仍读 crates/ 路径并以 ENOENT 失败。\n' +
        'for (const packKey of Object.keys(domainPackSourceByPlugin)) {\n' +
        '  domainPackSourceByPlugin[packKey] = path.join(\n' +
        '    workspaceRoot, "plugin-" + pluginName, "domain_packs",\n' +
        '  );\n' +
        '}\n' +
        '// 独立单插件仓没有其它插件的域包源码；owner 冲突检查只读本仓随包资产。\n' +
        'for (const key of Object.keys(domainPackSourceByPlugin)) {\n' +
        '  if (key !== pluginName) delete domainPackSourceByPlugin[key];\n' +
        '}\n' +
        '// 必须在映射重写之后再取 domainPackSource：旧版在重写前取值会留下 crates/ 路径，\n' +
        '// Windows Release --check 实测报「域包清单目录不存在：...\\crates\\plugin-opc-accounting\\domain_packs」。\n' +
        'const domainPackSource = domainPackSourceByPlugin[pluginName] ?? null;',
    );
  writeFileSync(join(repo, "scripts", "build-plugin-bundle.mjs"), relocated);
}

function writeReleaseDependencyCheck(repo, name) {
  writeFileSync(
    join(repo, "scripts", "check-release-dependencies.mjs"),
    `#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const manifests = [];

function visit(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.name === ".git" || entry.name === "target" || entry.name === "dist" || entry.name === "node_modules") {
      continue;
    }
    const full = join(dir, entry.name);
    if (entry.isDirectory()) visit(full);
    else if (entry.isFile() && entry.name === "Cargo.toml") manifests.push(full);
  }
}

visit(ROOT);
const violations = [];
for (const manifest of manifests) {
  const text = readFileSync(manifest, "utf8");
  for (const match of text.matchAll(/^\\s*([a-zA-Z0-9_-]+)\\s*=\\s*\\{([^}]*)\\}\\s*$/gm)) {
    const dependency = match[1];
    const body = match[2];
    const pathValue = /(?:^|,)\\s*path\\s*=\\s*"([^"]+)"/.exec(body)?.[1];
    if (!pathValue) continue;
    const target = resolve(dirname(manifest), pathValue);
    const targetRelative = relative(ROOT, target);
    if (targetRelative === ".." || targetRelative.startsWith(\`..\${sep}\`) || resolve(target) === resolve(ROOT)) {
      violations.push(\`\${relative(ROOT, manifest)}: \${dependency} path 越出独立仓：\${pathValue}\`);
    }
  }
  if (/path\\s*=\\s*"[^"]*AxInvest[^"]*"/.test(text)) {
    violations.push(\`\${relative(ROOT, manifest)}: 仍指向主仓路径\`);
  }
}

// 版本一致性：源只有三处 —— .axinvest-plugin-repos.json 的 pluginVersion/rustVersion、
// plugin-ci.yml 的 env.PLUGIN_VERSION、各 Cargo.toml 的 [package] version。
// 任何一处漂移都必须在这里先红，别等打包产物才发现。
const NL = String.fromCharCode(10);
const releaseFile = join(ROOT, ".axinvest-plugin-repos.json");
const ciFile = join(ROOT, ".github", "workflows", "plugin-ci.yml");
if (existsSync(releaseFile)) {
  const meta = JSON.parse(readFileSync(releaseFile, "utf8"));
  const declared = [];
  if (typeof meta.pluginVersion === "string") {
    declared.push([".axinvest-plugin-repos.json pluginVersion", meta.pluginVersion]);
  }
  if (typeof meta.rustVersion === "string") {
    declared.push([".axinvest-plugin-repos.json rustVersion", meta.rustVersion]);
  }
  const ciLines = existsSync(ciFile) ? readFileSync(ciFile, "utf8").split(NL) : [];
  const ciLine = ciLines.find((line) => line.trim().startsWith("PLUGIN_VERSION:"));
  declared.push([
    "plugin-ci.yml env.PLUGIN_VERSION",
    ciLine
      ? ciLine.slice(ciLine.indexOf(":") + 1).trim().split('"').join("").split("'").join("")
      : "<missing>",
  ]);
  for (const manifest of manifests) {
    const lines = readFileSync(manifest, "utf8").split(NL);
    const start = lines.findIndex((line) => line.trim() === "[package]");
    if (start < 0) continue;
    for (let i = start + 1; i < lines.length; i += 1) {
      const line = lines[i].trim();
      if (line.startsWith("[")) break;
      if (line.startsWith("version")) {
        const value = line.slice(line.indexOf('"') + 1, line.lastIndexOf('"'));
        if (value) declared.push([relative(ROOT, manifest), value]);
        break;
      }
    }
  }
  const expected = meta.pluginVersion;
  const drift = declared.filter((entry) => entry[1] !== expected);
  if (drift.length) {
    violations.push(
      "版本不一致（应全部为 " + expected + "）：" + NL
        + declared.map((entry) => "  " + entry[0] + " = " + entry[1]).join(NL),
    );
  }
}

if (violations.length) {
  console.error(violations.join("\\n"));
  process.exit(1);
}
console.log(\`PLUGIN_RELEASE_DEPENDENCIES_OK ${name}: \${manifests.length} manifests\`);
`,
  );
}

function writeGithubWorkflows(repo, name, exampleVersion) {
  const hasRustWorker = Boolean(WORKER_SPECS[name]);
  const rustToolchain = hasRustWorker
    ? `      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: "${RUST_TOOLCHAIN}"
          components: rustfmt, clippy
`
    : "";
  const rustChecks = hasRustWorker
    ? `      - name: Check Rust formatting
        run: cargo fmt --all --check
      - name: Check Rust sources
        run: cargo check --workspace --all-targets
      - name: Test Rust sources
        run: cargo test --workspace --all-targets
      - name: Lint Rust sources
        run: cargo clippy --workspace --all-targets -- -D warnings
`
    : "";
  mkdirSync(join(repo, ".github", "workflows"), { recursive: true });
  const pluginVersion = hasRustWorker ? RUST_RELEASE_VERSION : DECLARATIVE_RELEASE_VERSION;
  writeFileSync(
    join(repo, ".github", "workflows", "plugin-ci.yml"),
    `name: Plugin CI

on:
  push:
    branches: [master, main]
  pull_request:
  workflow_dispatch:

permissions:
  contents: read

env:
  CARGO_INCREMENTAL: "0"
  CARGO_PROFILE_DEV_DEBUG: "0"
  RUST_BACKTRACE: short
  PLUGIN_NAME: ${name}
  PLUGIN_VERSION: ${pluginVersion}

jobs:
  verify:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v5
      - uses: actions/setup-node@v5
        with:
          node-version: "24"
${rustToolchain}      - name: Install bundle-only Node dependencies
        if: hashFiles('package.json') != ''
        run: npm install --no-audit --no-fund
      - name: Check independent repository boundaries
        run: node scripts/check-release-dependencies.mjs
${rustChecks}      - name: Build and verify plugin bundle
        run: node scripts/build-plugin-bundle.mjs --plugin "\${PLUGIN_NAME}" --plugin-version "\${PLUGIN_VERSION}" --check --out "dist/plugins/\${PLUGIN_NAME}"
`,
  );
  // The second workflow is the release workflow; keep the CI template above intact.
  writeFileSync(
    join(repo, ".github", "workflows", "release-plugin.yml"),
    `name: Release Plugin

on:
  push:
    tags:
      - "v*"
  workflow_dispatch:
    inputs:
      release_tag:
        description: Existing tag, for example v${exampleVersion}
        required: true
        type: string

permissions:
  contents: write

env:
  CARGO_INCREMENTAL: "0"
  CARGO_PROFILE_DEV_DEBUG: "0"
  RUST_BACKTRACE: short
  PLUGIN_NAME: ${name}
  RELEASE_TAG: \${{ github.event_name == 'workflow_dispatch' && inputs.release_tag || github.ref_name }}

jobs:
  build:
    name: Build (\${{ matrix.os }}/\${{ matrix.arch }})
    runs-on: \${{ matrix.runner }}
    strategy:
      fail-fast: true
      matrix:
        include:
          - os: linux
            arch: x86_64
            runner: ubuntu-24.04
            target: x86_64-unknown-linux-gnu
          - os: linux
            arch: aarch64
            runner: ubuntu-24.04-arm
            target: aarch64-unknown-linux-gnu
          - os: macos
            arch: x86_64
            runner: macos-15-intel
            target: x86_64-apple-darwin
          - os: macos
            arch: aarch64
            runner: macos-15
            target: aarch64-apple-darwin
          - os: windows
            arch: x86_64
            runner: windows-2025
            target: x86_64-pc-windows-msvc
          - os: windows
            arch: aarch64
            runner: windows-11-arm
            target: aarch64-pc-windows-msvc
    steps:
      - uses: actions/checkout@v5
        with:
          ref: \${{ env.RELEASE_TAG }}
      - uses: actions/setup-node@v5
        with:
          node-version: "24"
${rustToolchain}      - name: Install bundle-only Node dependencies
        if: hashFiles('package.json') != ''
        run: npm install --no-audit --no-fund
      - name: Check independent repository boundaries
        run: node scripts/check-release-dependencies.mjs
${rustChecks}      - name: Build, verify, and package release bundle
        shell: bash
        run: |
          set -euo pipefail
          artifact_url="\${GITHUB_SERVER_URL}/\${GITHUB_REPOSITORY}/releases/download/\${RELEASE_TAG}/\${PLUGIN_NAME}-\${{ matrix.os }}-\${{ matrix.arch }}.tar.gz"
          node scripts/build-plugin-bundle.mjs --plugin "\${PLUGIN_NAME}" --plugin-version "\${RELEASE_TAG#v}" --release --target "\${{ matrix.target }}" --check --out "dist/plugins/\${PLUGIN_NAME}"
          node - <<'NODE'
          const fs = require("node:fs");
          const manifest = JSON.parse(fs.readFileSync("dist/plugins/" + process.env.PLUGIN_NAME + "/plugin.json", "utf8"));
          const expected = "v" + manifest.version;
          if (process.env.RELEASE_TAG !== expected) {
            console.error("release tag " + process.env.RELEASE_TAG + " does not match plugin version " + expected);
            process.exit(1);
          }
          NODE
          node generate-plugin-release-manifest.mjs \\
            --bundle "dist/plugins/\${PLUGIN_NAME}" \\
            --artifact-url "\${artifact_url}" \\
            --os "\${{ matrix.os }}" \\
            --arch "\${{ matrix.arch }}" \\
            --out "dist/release/\${{ matrix.os }}-\${{ matrix.arch }}"
          node generate-plugin-release-manifest.mjs \\
            --check \\
            --bundle "dist/plugins/\${PLUGIN_NAME}" \\
            --artifact-url "\${artifact_url}" \\
            --os "\${{ matrix.os }}" \\
            --arch "\${{ matrix.arch }}" \\
            --out "dist/release/\${{ matrix.os }}-\${{ matrix.arch }}"
      - uses: actions/upload-artifact@v4
        with:
          name: release-\${{ matrix.os }}-\${{ matrix.arch }}
          path: dist/release/\${{ matrix.os }}-\${{ matrix.arch }}
          if-no-files-found: error

  publish:
    name: Publish verified release
    needs: build
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v5
        with:
          ref: \${{ env.RELEASE_TAG }}
      - uses: actions/setup-node@v5
        with:
          node-version: "24"
      - uses: actions/download-artifact@v4
        with:
          pattern: release-*
          path: dist/platforms
          merge-multiple: false
      - name: Build release manifest and stage platform archives
        shell: bash
        run: |
          set -euo pipefail
          args=()
          for release in dist/platforms/release-*/release.json; do
            args+=(--artifact "\${release}")
          done
          args+=(--require-platform linux/x86_64)
          args+=(--require-platform linux/aarch64)
          args+=(--require-platform macos/x86_64)
          args+=(--require-platform macos/aarch64)
          args+=(--require-platform windows/x86_64)
          args+=(--require-platform windows/aarch64)
          node generate-plugin-release-manifest.mjs --merge \\
            --version "\${RELEASE_TAG#v}" \\
            --out dist/release \\
            --stage-artifacts \\
            "\${args[@]}"
      - name: Create or update GitHub Release
        env:
          GH_TOKEN: \${{ github.token }}
        shell: bash
        run: |
          if gh release view "\${RELEASE_TAG}" --repo "\${GITHUB_REPOSITORY}" >/dev/null 2>&1; then
            gh release upload "\${RELEASE_TAG}" \\
              dist/release/* \\
              --repo "\${GITHUB_REPOSITORY}" \\
              --clobber
          else
            gh release create "\${RELEASE_TAG}" \\
              dist/release/* \\
              --repo "\${GITHUB_REPOSITORY}" \\
              --title "\${PLUGIN_NAME} \${RELEASE_TAG}" \\
              --generate-notes
          fi
`,
  );
  return;
  writeFileSync(
    join(repo, ".github", "workflows", "plugin-ci.yml"),
    `name: Plugin CI

on:
  push:
    branches: [master, main]
  pull_request:
  workflow_dispatch:

permissions:
  contents: read

env:
  CARGO_INCREMENTAL: "0"
  CARGO_PROFILE_DEV_DEBUG: "0"
  RUST_BACKTRACE: short
  PLUGIN_NAME: ${name}

jobs:
  verify:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v5
      - uses: actions/setup-node@v5
        with:
          node-version: "24"
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: "1.97.0"
          components: rustfmt, clippy
      - name: Build and verify plugin bundle
        run: node scripts/build-plugin-bundle.mjs --plugin "\${PLUGIN_NAME}" --check --out "dist/plugins/\${PLUGIN_NAME}"
`,
  );
  writeFileSync(
    join(repo, ".github", "workflows", "release-plugin.yml"),
    `name: Release Plugin

on:
  push:
    tags:
      - "v*"
  workflow_dispatch:
    inputs:
      release_tag:
        description: Existing tag, for example v${exampleVersion}
        required: true
        type: string

permissions:
  contents: write

env:
  CARGO_INCREMENTAL: "0"
  CARGO_PROFILE_DEV_DEBUG: "0"
  RUST_BACKTRACE: short
  PLUGIN_NAME: ${name}
  RELEASE_TAG: \${{ github.event_name == 'workflow_dispatch' && inputs.release_tag || github.ref_name }}

jobs:
  build:
    name: Build (\${{ matrix.os }}/\${{ matrix.arch }})
    runs-on: \${{ matrix.runner }}
    strategy:
      fail-fast: true
      matrix:
        include:
          - os: linux
            arch: x86_64
            runner: ubuntu-24.04
            target: x86_64-unknown-linux-gnu
          - os: linux
            arch: aarch64
            runner: ubuntu-24.04-arm
            target: aarch64-unknown-linux-gnu
          - os: macos
            arch: x86_64
            runner: macos-15-intel
            target: x86_64-apple-darwin
          - os: macos
            arch: aarch64
            runner: macos-15
            target: aarch64-apple-darwin
          - os: windows
            arch: x86_64
            runner: windows-2025
            target: x86_64-pc-windows-msvc
          - os: windows
            arch: aarch64
            runner: windows-11-arm
            target: aarch64-pc-windows-msvc
    steps:
      - uses: actions/checkout@v5
        with:
          ref: \${{ env.RELEASE_TAG }}
      - uses: actions/setup-node@v5
        with:
          node-version: "24"
${rustToolchain}      - name: Install bundle-only Node dependencies
        if: hashFiles('package.json') != ''
        run: npm install --no-audit --no-fund
      - name: Check independent repository boundaries
        run: node scripts/check-release-dependencies.mjs
${rustChecks}      - name: Build, verify, and package release bundle
        shell: bash
        run: |
          set -euo pipefail
          artifact_url="\${GITHUB_SERVER_URL}/\${GITHUB_REPOSITORY}/releases/download/\${RELEASE_TAG}/\${PLUGIN_NAME}-\${{ matrix.os }}-\${{ matrix.arch }}.tar.gz"
          node scripts/build-plugin-bundle.mjs --plugin "\${PLUGIN_NAME}" --release --target "\${{ matrix.target }}" --check --out "dist/plugins/\${PLUGIN_NAME}"
          node - <<'NODE'
          const fs = require("node:fs");
          const manifest = JSON.parse(fs.readFileSync("dist/plugins/" + process.env.PLUGIN_NAME + "/plugin.json", "utf8"));
          const expected = "v" + manifest.version;
          if (process.env.RELEASE_TAG !== expected) {
            console.error("release tag " + process.env.RELEASE_TAG + " does not match plugin version " + expected);
            process.exit(1);
          }
          NODE
          node generate-plugin-release-manifest.mjs \\
            --bundle "dist/plugins/\${PLUGIN_NAME}" \\
            --artifact-url "\${artifact_url}" \\
            --os "\${{ matrix.os }}" \\
            --arch "\${{ matrix.arch }}" \\
            --out "dist/release/\${{ matrix.os }}-\${{ matrix.arch }}"
          node generate-plugin-release-manifest.mjs \\
            --check \\
            --bundle "dist/plugins/\${PLUGIN_NAME}" \\
            --artifact-url "\${artifact_url}" \\
            --os "\${{ matrix.os }}" \\
            --arch "\${{ matrix.arch }}" \\
            --out "dist/release/\${{ matrix.os }}-\${{ matrix.arch }}"
      - uses: actions/upload-artifact@v4
        with:
          name: release-\${{ matrix.os }}-\${{ matrix.arch }}
          path: dist/release/\${{ matrix.os }}-\${{ matrix.arch }}
          if-no-files-found: error

  publish:
    name: Publish verified release
    needs: build
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v5
        with:
          ref: \${{ env.RELEASE_TAG }}
      - uses: actions/setup-node@v5
        with:
          node-version: "24"
      - uses: actions/download-artifact@v4
        with:
          pattern: release-*
          path: dist/platforms
          merge-multiple: false
      - name: Build release manifest and stage platform archives
        shell: bash
        run: |
          set -euo pipefail
          args=()
          for release in dist/platforms/release-*/release.json; do
            args+=(--artifact "\${release}")
          done
          args+=(--require-platform linux/x86_64)
          args+=(--require-platform linux/aarch64)
          args+=(--require-platform macos/x86_64)
          args+=(--require-platform macos/aarch64)
          args+=(--require-platform windows/x86_64)
          args+=(--require-platform windows/aarch64)
          node generate-plugin-release-manifest.mjs --merge \\
            --version "\${RELEASE_TAG#v}" \\
            --out dist/release \\
            --stage-artifacts \\
            "\${args[@]}"
      - name: Create or update GitHub Release
        env:
          GH_TOKEN: \${{ github.token }}
        shell: bash
        run: |
          if gh release view "\${RELEASE_TAG}" --repo "\${GITHUB_REPOSITORY}" >/dev/null 2>&1; then
            gh release upload "\${RELEASE_TAG}" \\
              dist/release/* \\
              --repo "\${GITHUB_REPOSITORY}" \\
              --clobber
          else
            gh release create "\${RELEASE_TAG}" \\
              dist/release/* \\
              --repo "\${GITHUB_REPOSITORY}" \\
              --title "\${PLUGIN_NAME} \${RELEASE_TAG}" \\
              --generate-notes
          fi
`,
  );
}

function writeRepo(name, info) {
  const repo = join(OUT_ROOT, name);
  const version = requestedVersion();
  if (SYNC_EXISTING) {
    // 只清理由生成器拥有的内容；.git / dist / target / node_modules 一律不碰。
    // 这是与默认 --bootstrap-from-main 最本质的区别：后者会整目录 rmSync（连 .git）。
    if (!existsSync(join(repo, ".git"))) fail(`--sync-existing 目标不是 Git 仓：${repo}`);
    for (const entry of readdirSync(repo, { withFileTypes: true })) {
      if (SYNC_PROTECTED.has(entry.name)) continue;
      rmSync(join(repo, entry.name), { recursive: true, force: true });
    }
  } else {
    rmSync(repo, { recursive: true, force: true });
  }
  mkdirSync(repo, { recursive: true });
  const detail = info.packageName
    ? generateWorkerRepo(name, repo, version)
    : generateDeclarativeRepo(name, repo, version);
  copyTree("axagent-plugins/scripts/opc-domain-pack-plugins.json", join(repo, "scripts", "opc-domain-pack-plugins.json"));
  bundleScript(repo, name);
  copyTree("scripts/generate-plugin-release-manifest.mjs", join(repo, "generate-plugin-release-manifest.mjs"));
  writeReleaseDependencyCheck(repo, name);
  writeGithubWorkflows(repo, name, version);
  // 这些脚本由生成器从主仓整段复制到独立插件仓，并非本仓原生源码。
  // 不标 linguist-vendored 时，单是 build-plugin-bundle.mjs（约 270 KB）就会
  // 把纯声明式插件仓的 GitHub 语言标签压成 99% JavaScript。
  writeFileSync(
    join(repo, ".gitattributes"),
    [
      "# 由主仓生成器复制的 vendored 脚本，不计入 GitHub Linguist 语言统计。",
      "scripts/build-plugin-bundle.mjs linguist-vendored",
      "scripts/check-release-dependencies.mjs linguist-vendored",
      "generate-plugin-release-manifest.mjs linguist-vendored",
      "",
    ].join("\n"),
  );
  writeFileSync(
    join(repo, ".gitignore"),
    ["node_modules/", "target/", "dist/", ""].join("\n"),
  );
  if (detail.packageName) {
    // 裸跑 cargo 也必须落在 CI 锁定的 1.97.0，避免本机默认工具链给出假结果。
    writeFileSync(
      join(repo, "rust-toolchain.toml"),
      ['[toolchain]', 'channel = "' + RUST_TOOLCHAIN + '"', 'components = ["rustfmt", "clippy"]', 'profile = "minimal"', ""].join("\n"),
    );
  }
  writeFileSync(
    join(repo, RELEASE_REPOS_FILE),
    JSON.stringify(
      {
        owner: RELEASE_OWNER,
        repo: releaseRepoName(name),
        plugin: name,
        rustVersion: version,
        pluginVersion: info.version,
        releaseManifestUrl: releaseManifestUrl(name),
      },
      null,
      2,
    ) + "\n",
  );
  writeFileSync(
    join(repo, "README.md"),
    `# ${name}\n\n` +
      `本目录由 \`scripts/generate-plugin-independent-repos.mjs\` 从 AxInvest 主仓非破坏性生成。\n\n` +
      `- 插件名：${name}\n` +
      `- Cargo 包：${detail.packageName ?? "无（声明式插件）"}\n` +
      `- SDK 依赖：副本 ${detail.sdkDirs.join("、") || "无（声明式插件）"}\n` +
      "- CI：`.github/workflows/plugin-ci.yml`\n" +
      "- 发布：推送与 `plugin.json` 版本一致的 `v*` tag，由 `.github/workflows/release-plugin.yml` 生成 GitHub Release\n" +
      `- 主应用发布地址：\`${releaseManifestUrl(name)}\`\n` +
      `- 打包：\`node scripts/build-plugin-bundle.mjs --plugin ${name} --check\`\n` +
      `- 单平台发布清单：\`node generate-plugin-release-manifest.mjs --bundle dist/plugins/${name} --artifact-url https://example.com/${name}-linux-x86_64.tar.gz --os linux --arch x86_64\`\n` +
      "- 正式发布：`release-plugin.yml` 在六个平台分别生成清单，再由 `--merge --require-platform` 汇总为唯一 `release.json` 和全部平台包。\n" +
      "- SDK：`sdk/` 是生成器从主仓同步的最小源码快照，所有依赖都留在本仓内；无需预先发布 crates.io。\n",
  );
  // worker 仓必须在首次 tag 前自身通过 rustfmt；声明式仓没有 Rust target，
  // 不能把 cargo fmt 的 “Failed to find targets” 误判成格式违规。
  if (detail.crate) {
    run("cargo", ["fmt", "--all"], repo, `${name} cargo fmt`);
    const fmtCheck = shell("cargo", ["fmt", "--all", "--check"], repo);
    if (!fmtCheck.ok) {
      fail(`${name} 独立仓 rustfmt 后仍不干净：${fmtCheck.output.split("\n").pop() ?? ""}`);
    }
  }
  if (name === "stock" && !SKIP_GIT) {
    run("npm", ["install", "--no-audit", "--no-fund"], repo, `${name} npm install`);
  }
  if (!SKIP_GIT) {
    run("git", ["init", "-q"], repo, `${name} git init`);
    run("git", ["add", "."], repo, `${name} git add`);
    run("git", ["-c", "user.name=Codex", "-c", "user.email=codex@example.com", "commit", "-qm", `chore: snapshot ${name} independent repo`], repo, `${name} git commit`);
    // --check 会生成 Cargo.lock；把它固定在快照里，避免验收后独立仓变脏。
    const check = shell(process.execPath, ["scripts/build-plugin-bundle.mjs", "--plugin", name, "--check"], repo);
    if (!check.ok) fail(`${name} 初始 bundle 自检失败：${check.output.split("\n").pop() ?? ""}`);
    const lockfile = join(repo, "Cargo.lock");
    if (existsSync(lockfile)) {
      run("git", ["add", "Cargo.lock"], repo, `${name} git add Cargo.lock`);
      run("git", ["-c", "user.name=Codex", "-c", "user.email=codex@example.com", "commit", "-qm", `chore: lock ${name} snapshot dependencies`], repo, `${name} git commit lockfile`);
    }
  }
  if (!SKIP_GIT) {
    const builtManifest = JSON.parse(
      readFileSync(join(repo, "dist", "plugins", name, "plugin.json"), "utf8"),
    );
    const metadataPath = join(repo, RELEASE_REPOS_FILE);
    const metadata = JSON.parse(readFileSync(metadataPath, "utf8"));
    metadata.pluginVersion = builtManifest.version;
    writeFileSync(metadataPath, JSON.stringify(metadata, null, 2) + "\n");
    commitIfChanged(repo, `chore: record ${name} release version`, `${name} release version`);
  }
  return repo;
}

/** 只枚举生成器拥有的文件；Symlink 与受保护目录不进快照。 */
function snapshotTree(dir, base = dir, out = new Map()) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (SYNC_PROTECTED.has(entry.name)) continue;
    const full = join(dir, entry.name);
    const rel = relative(base, full).split(sep).join("/");
    if (entry.isDirectory()) snapshotTree(full, base, out);
    else if (entry.isFile()) out.set(rel, readFileSync(full).toString("base64"));
  }
  return out;
}

function dryRunSync(name, info) {
  const realRepo = join(OUT_ROOT, name);
  if (!existsSync(join(realRepo, ".git"))) fail(`--dry-run 目标不是 Git 仓：${realRepo}`);
  const tempRoot = mkdtempSync(join(tmpdir(), `axinvest-plugin-sync-${name}-`));
  const shadow = join(tempRoot, name);
  mkdirSync(shadow, { recursive: true });
  // 复制受管文件到影子目录；.git / dist / target / node_modules 不参与 diff。
  cpSync(realRepo, shadow, {
    recursive: true,
    force: true,
    verbatimSymlinks: true,
    filter: (src) => !SYNC_PROTECTED.has(src.split(/[\\/]/).pop()),
  });
  // writeRepo 的 sync 分支要求目标是 Git 仓；影子目录放一个空 .git 占位即可，
  // snapshotTree 会跳过它，也不会有任何真实 Git 数据被读或写。
  mkdirSync(join(shadow, ".git"), { recursive: true });
  const before = snapshotTree(realRepo);
  const previousOut = OUT_ROOT;
  try {
    OUT_ROOT = tempRoot;
    writeRepo(name, info);
    const after = snapshotTree(shadow);
    const added = [...after.keys()].filter((k) => !before.has(k)).sort();
    const removed = [...before.keys()].filter((k) => !after.has(k)).sort();
    const changed = [...after.keys()].filter((k) => before.has(k) && before.get(k) !== after.get(k)).sort();
    console.log(JSON.stringify({ plugin: name, target: realRepo, added, removed, changed }, null, 2));
    console.log(
      `PLUGIN_REPOS_SYNC_DRYRUN ${name}: +${added.length} -${removed.length} ~${changed.length}（未写入真实仓）`,
    );
  } finally {
    OUT_ROOT = previousOut;
    rmSync(tempRoot, { recursive: true, force: true });
  }
}

if (DRY_RUN && plugins.length !== 1) fail("--dry-run 只允许单插件");

const summary = [];
for (const name of plugins) {
  const info = CONTRACT_BY_NAME.get(name);
  if (!info) fail(`契约缺少插件条目 ${name}`);
  if (DRY_RUN) {
    dryRunSync(name, info);
    continue;
  }
  const repo = writeRepo(name, info);
  summary.push({ plugin: name, repo, packageName: info.packageName });
  console.log(`[plugin-repos] ${name}: ${repo}`);
}
if (!DRY_RUN) {
  console.log(JSON.stringify({ outputRoot: OUT_ROOT, plugins: summary }, null, 2));
  console.log("PLUGIN_REPOS_GENERATED");
}
