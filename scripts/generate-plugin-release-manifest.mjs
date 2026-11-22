#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 生成/校验插件发布地址所需的 `release.json` + `.tar.gz` 成品包。
 *
 * 输入是 `build-plugin-bundle.mjs` 的已验收插件目录；输出落在该目录的 `release/` 子目录：
 *   - `<plugin>.tar.gz`：插件目录内容（不含 release/ 自身），仅 HTTPS 分发；
 *   - `release.json`：默认写出 `{ version, artifacts: [{ os, arch, artifactUrl, sha256 }] }`；
 *     也兼容生成旧版 `{ artifactUrl, sha256, version }`。sha256 是解压后目录规范化哈希，
 *     口径必须与 Rust `hash_plugin_directory` 一致，否则安装期会拒绝。
 *
 * `artifactUrl` 必须由 `--artifact-url https://...` 显式提供；生成器不猜 CDN 地址。
 * 与 Rust 门禁对齐：只接受 HTTPS `.tar.gz` / `.tgz`，不接受 HTTP / file / 本地路径。
 *
 * 用法：
 *   node scripts/generate-plugin-release-manifest.mjs \
 *     --bundle dist/plugins/wiki --artifact-url https://example.com/wiki.tar.gz \
 *     --os linux --arch x86_64 [--out dist/releases/wiki]
 *   node scripts/generate-plugin-release-manifest.mjs \
 *     --bundle dist/plugins/wiki --artifact-url https://example.com/wiki.tar.gz \
 *     --legacy [--out dist/releases/wiki]
 *   node scripts/generate-plugin-release-manifest.mjs --merge \
 *     --version 3.0.0 \
 *     --artifact dist/release/linux-x86_64/release.json [--out dist/release]
 *   node scripts/generate-plugin-release-manifest.mjs --selftest
 *   node scripts/generate-plugin-release-manifest.mjs --check \
 *     --bundle dist/plugins/wiki --artifact-url https://example.com/wiki.tar.gz \
 *     --os linux --arch x86_64
 */
import { createHash } from "node:crypto";
import { gzipSync } from "node:zlib";
import {
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { basename, dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
// 主仓脚本位于 scripts/，独立插件仓则把同一脚本放在仓根。
const ROOT = basename(SCRIPT_DIR) === "scripts" ? resolve(SCRIPT_DIR, "..") : SCRIPT_DIR;
const argv = process.argv.slice(2);

function fail(message) {
  console.error(`[plugin-release] ${message}`);
  process.exit(1);
}

function flag(name) {
  const index = argv.indexOf(name);
  return index >= 0 ? argv[index + 1] ?? "" : null;
}

let selftestMode = false;
const SUPPORTED_OS = new Set(["linux", "macos", "windows"]);
const SUPPORTED_ARCH = new Set(["x86_64", "aarch64"]);

/** Rust `hash_plugin_directory`：相对路径按字节排序，路径与内容依次喂给 SHA-256。 */
function directoryHash(root) {
  const files = [];
  function visit(dir) {
    for (const entry of readdirSync(dir, { withFileTypes: true }).sort((a, b) => a.name < b.name ? -1 : a.name > b.name ? 1 : 0)) {
      const full = join(dir, entry.name);
      if (entry.isDirectory()) {
        visit(full);
      } else if (entry.isFile()) {
        files.push(relative(root, full).split("\\").join("/"));
      } else {
        fail(`发布目录不允许符号链接或特殊条目：${relative(ROOT, full)}`);
      }
    }
  }
  visit(root);
  files.sort();
  const hash = createHash("sha256");
  for (const file of files) {
    hash.update(file);
    hash.update(readFileSync(join(root, file)));
  }
  return hash.digest("hex");
}

function assertArtifactUrl(url) {
  if (selftestMode) {
    const valid = typeof url === "string" && /^https:\/\/\S+$/.test(url) && /\.(tar\.gz|tgz)$/.test(url);
    if (!valid) throw new Error("artifactUrl 必须是 HTTPS .tar.gz 或 .tgz");
    return;
  }
  if (!url || !/^https:\/\/\S+$/.test(url) || !/\.(tar\.gz|tgz)$/.test(url)) {
    fail("artifactUrl 必须是 HTTPS .tar.gz 或 .tgz");
  }
}

function normalizePlatform(os, arch) {
  const normalizedOs = (os ?? "").trim().toLowerCase();
  const normalizedArch = (arch ?? "").trim().toLowerCase();
  if (!SUPPORTED_OS.has(normalizedOs)) {
    throw new Error(`os 必须是 linux / macos / windows，实得 ${JSON.stringify(os)}`);
  }
  if (!SUPPORTED_ARCH.has(normalizedArch)) {
    throw new Error(`arch 必须是 x86_64 / aarch64，实得 ${JSON.stringify(arch)}`);
  }
  return { os: normalizedOs, arch: normalizedArch };
}

function validateArtifactEntry(entry, label = "artifact") {
  if (!entry || typeof entry !== "object") throw new Error(`${label} 必须是对象`);
  const platform = normalizePlatform(entry.os, entry.arch);
  assertArtifactUrl(entry.artifactUrl);
  if (!/^[0-9a-f]{64}$/.test(entry.sha256 ?? "")) {
    throw new Error(`${label} sha256 必须是 64 位小写十六进制`);
  }
  return {
    os: platform.os,
    arch: platform.arch,
    artifactUrl: entry.artifactUrl,
    sha256: entry.sha256,
  };
}

function collectMergeArtifacts() {
  const sources = [];
  const required = [];
  for (let index = 0; index < argv.length; index += 1) {
    if (argv[index] === "--artifact") {
      sources.push(resolve(ROOT, argv[index + 1]));
    } else if (argv[index] === "--require-platform") {
      required.push(normalizePlatform(...(argv[index + 1] ?? "").split("/")));
    }
  }
  return {
    sources,
    required: required.map(({ os, arch }) => `${os}/${arch}`),
  };
}

function createTarGz(source, archive) {
  const parent = dirname(archive);
  mkdirSync(parent, { recursive: true });
  // 用 Node 内置 zlib 写 gzip，避免依赖外部 tar/gzip；tar 头手工生成且只保留普通文件条目。
  const entries = [];
  function visit(dir) {
    for (const entry of readdirSync(dir, { withFileTypes: true }).sort((a, b) => a.name < b.name ? -1 : a.name > b.name ? 1 : 0)) {
      const full = join(dir, entry.name);
      if (entry.isDirectory()) {
        entries.push({ name: `${relative(source, full).split("\\").join("/")}/`, data: Buffer.alloc(0), type: 0x35 });
        visit(full);
      } else if (entry.isFile()) {
        entries.push({ name: relative(source, full).split("\\").join("/"), data: readFileSync(full), type: 0x30 });
      } else {
        fail(`发布目录不允许符号链接或特殊条目：${relative(ROOT, full)}`);
      }
    }
  }
  visit(source);
  const chunks = [];
  for (const entry of entries) {
    const header = Buffer.alloc(512, 0);
    header.write(entry.name, 0, 100, "utf8");
    header.write("0000644\0", 100, 8, "ascii");
    header.write("0000000\0", 108, 8, "ascii");
    header.write("0000000\0", 116, 8, "ascii");
    header.write(entry.data.length.toString(8).padStart(11, "0"), 124, 12, "ascii");
    header.write("00000000000\0", 136, 12, "ascii");
    header.fill(0x20, 148, 156);
    header[156] = entry.type;
    header.write("ustar\0", 257, 6, "ascii");
    header.write("00", 263, 2, "ascii");
    let checksum = 0;
    for (let i = 0; i < 512; i += 1) checksum += i >= 148 && i < 156 ? 0x20 : header[i];
    header.write(checksum.toString(8).padStart(6, "0"), 148, 6, "ascii");
    header[154] = 0;
    header[155] = 0x20;
    chunks.push(header, entry.data);
    const padding = (512 - (entry.data.length % 512)) % 512;
    if (padding) chunks.push(Buffer.alloc(padding, 0));
  }
  chunks.push(Buffer.alloc(1024, 0));
  writeFileSync(archive, gzipSync(Buffer.concat(chunks)));
}

function selftest() {
  const cases = [
    ["https://cdn.example.com/wiki.tar.gz", true],
    ["https://cdn.example.com/wiki.tgz", true],
    ["http://cdn.example.com/wiki.tar.gz", false],
    ["file:///tmp/wiki.tar.gz", false],
    ["https://cdn.example.com/wiki.zip", false],
    ["", false],
  ];
  for (const [url, expected] of cases) {
    let actual = true;
    try {
      assertArtifactUrl(url);
    } catch {
      actual = false;
    }
    if (actual !== expected) fail(`artifactUrl 判定不符：${url}`);
  }
  const platforms = [
    ["linux", "x86_64", true],
    ["windows", "amd64", false],
    ["macos", "arm64", false],
    ["darwin", "x86_64", false],
  ];
  for (const [os, arch, expected] of platforms) {
    let actual = true;
    try {
      normalizePlatform(os, arch);
    } catch {
      actual = false;
    }
    if (actual !== expected) fail(`平台判定不符：${os}/${arch}`);
  }
  console.log("PASS: 发布地址只接受 HTTPS .tar.gz / .tgz；平台只接受 linux/macos/windows + x86_64/aarch64");
}

if (argv.includes("--selftest")) {
  selftestMode = true;
  selftest();
  process.exit(0);
}

const mergeMode = argv.includes("--merge");
const bundleFlag = flag("--bundle");
const artifactUrl = flag("--artifact-url");
if (!mergeMode && !bundleFlag) {
  fail("需要 --bundle <目录>");
}
let bundle = null;
let manifest = null;
if (bundleFlag) {
  bundle = resolve(ROOT, bundleFlag);
  if (!existsSync(join(bundle, "plugin.json"))) {
    fail(`插件目录缺少 plugin.json：${bundleFlag}`);
  }
  manifest = JSON.parse(readFileSync(join(bundle, "plugin.json"), "utf8"));
}

const version = flag("--version") ?? manifest?.version;
if (!version || typeof version !== "string") {
  fail("插件版本缺失");
}
const out = resolve(ROOT, flag("--out") ?? (bundle ? join(bundle, "release") : join(ROOT, "dist", "release")));
const releasePath = join(out, "release.json");
const check = argv.includes("--check");
const legacy = argv.includes("--legacy");

if (mergeMode) {
  const { sources, required } = collectMergeArtifacts();
  if (sources.length === 0) {
    fail("--merge 至少需要一个 --artifact <platform-release.json>");
  }
  const seen = new Set();
  const artifacts = sources
    .map((source) => {
      const input = JSON.parse(readFileSync(source, "utf8"));
      if (input.version !== version) {
        fail(`${relative(ROOT, source)} 的 version=${input.version} 与聚合版本 ${version} 不一致`);
      }
      const candidates = Array.isArray(input.artifacts)
        ? input.artifacts
        : [{ os: flag("--os"), arch: flag("--arch"), artifactUrl: input.artifactUrl, sha256: input.sha256 }];
      if (candidates.length !== 1) {
        fail(`${relative(ROOT, source)} 必须恰好描述一个平台产物`);
      }
      const artifact = validateArtifactEntry(candidates[0], relative(ROOT, source));
      const key = `${artifact.os}/${artifact.arch}`;
      if (seen.has(key)) fail(`平台产物重复：${key}`);
      seen.add(key);
      return { ...artifact, source };
    })
    .sort((a, b) => `${a.os}/${a.arch}`.localeCompare(`${b.os}/${b.arch}`));
  const missing = required.filter((platform) => !seen.has(platform));
  if (missing.length) {
    fail(`缺少必需平台产物：${missing.join(", ")}`);
  }
  mkdirSync(out, { recursive: true });
  if (argv.includes("--stage-artifacts")) {
    for (const artifact of artifacts) {
      const assetName = basename(new URL(artifact.artifactUrl).pathname);
      if (!assetName || !/\.(tar\.gz|tgz)$/.test(assetName)) {
        fail(`无法从 artifactUrl 取得合法发布包名：${artifact.artifactUrl}`);
      }
      const local = readdirSync(dirname(artifact.source))
        .filter((name) => /\.(tar\.gz|tgz)$/.test(name))
        .map((name) => join(dirname(artifact.source), name));
      if (local.length !== 1) {
        fail(`${relative(ROOT, dirname(artifact.source))} 必须恰好包含一个发布包，实得 ${local.length}`);
      }
      copyFileSync(local[0], join(out, assetName));
    }
  }
  const output = {
    version,
    artifacts: artifacts.map(({ source: _source, ...artifact }) => artifact),
  };
  writeFileSync(releasePath, `${JSON.stringify(output, null, 2)}\n`);
  console.log(`[plugin-release] 合并 ${artifacts.length} 个平台产物：${relative(ROOT, releasePath)}`);
  process.exit(0);
}

const archive = join(out, `${basename(bundle)}.tar.gz`);
if (!artifactUrl) {
  fail("需要 --artifact-url <https://.../*.tar.gz>");
}
assertArtifactUrl(artifactUrl);
const { os, arch } = normalizePlatform(flag("--os"), flag("--arch"));

if (!check) {
  const stage = join(out, ".stage");
  rmSync(out, { recursive: true, force: true });
  mkdirSync(stage, { recursive: true });
  for (const entry of readdirSync(bundle, { withFileTypes: true })) {
    if (entry.name === "release") continue;
    const source = join(bundle, entry.name);
    const target = join(stage, entry.name);
    cpSync(source, target, {
      recursive: entry.isDirectory(),
      errorOnExist: true,
      verbatimSymlinks: true,
    });
  }
  const sha256 = directoryHash(stage);
  createTarGz(stage, archive);
  rmSync(stage, { recursive: true, force: true });
  mkdirSync(out, { recursive: true });
  const release = legacy
    ? { artifactUrl, sha256, version }
    : { version, artifacts: [{ os, arch, artifactUrl, sha256 }] };
  writeFileSync(releasePath, `${JSON.stringify(release, null, 2)}\n`);
  console.log(`[plugin-release] 发布清单：${relative(ROOT, releasePath)}`);
  console.log(`[plugin-release] 成品包   ：${relative(ROOT, archive)}`);
  console.log(`[plugin-release] 平台     ：${os}/${arch}${legacy ? "（旧格式兼容）" : ""}`);
  console.log(`[plugin-release] sha256   ：${sha256}`);
  process.exit(0);
}

if (!existsSync(archive) || !existsSync(releasePath)) {
  fail(`发布产物不存在：${relative(ROOT, out)}（先不带 --check 运行生成器）`);
}
const release = JSON.parse(readFileSync(releasePath, "utf8"));
if (release.version !== version) {
  fail("release.json 的 version 与当前输入不一致");
}
if (legacy) {
  if (release.artifactUrl !== artifactUrl) {
    fail("旧格式 release.json 的 artifactUrl 与当前输入不一致");
  }
  validateArtifactEntry({ os, arch, artifactUrl: release.artifactUrl, sha256: release.sha256 });
} else {
  if (!Array.isArray(release.artifacts) || release.artifacts.length !== 1) {
    fail("平台格式 release.json 必须恰好声明一个 artifacts[] 条目");
  }
  const artifact = validateArtifactEntry(release.artifacts[0]);
  if (artifact.os !== os || artifact.arch !== arch || artifact.artifactUrl !== artifactUrl) {
    fail("release.json 的 os / arch / artifactUrl 与当前输入不一致");
  }
}
console.log(`PASS: ${relative(ROOT, releasePath)} 与 ${relative(ROOT, archive)} 形状一致`);
