#!/usr/bin/env node
// scripts/check-line-endings.mjs
//
// 行尾规范检查 —— 四层防线的最后一层（CI / pre-commit / 本地均可调用）。
//
//   1. 保存   → .editorconfig                  编辑器自动写 LF
//   2. 格式化 → dprint.json newLineKind=lf     `npm run format` 修正
//   3. 提交   → .gitattributes eol=lf          入库前规范化
//   4. 兜底   → 本脚本                         拦截漏网之鱼
//
// 规则：
//   - 所有文本文件必须 LF（禁止出现 \r\n）
//   - 唯一例外：.bat / .cmd / .ps1 必须 CRLF（cmd.exe 与 PowerShell 解析依赖）
//   - 二进制文件、生成物目录、外部数据源不检查
//
// 用法：
//   node scripts/check-line-endings.mjs          # 仅检查
//   node scripts/check-line-endings.mjs --fix    # 自动转换后退出 0
//   node scripts/check-line-endings.mjs --quiet  # 只打印汇总
//
// 说明：设计为单进程 Node 实现（与 i18n-scan.mjs 同构），避免 bash 逐文件起子 shell
//       在 Windows 上 fork 资源耗尽。

import { readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { extname, join, relative, sep } from "node:path";

const ROOT = process.cwd();
const args = new Set(process.argv.slice(2));
const FIX = args.has("--fix");
const QUIET = args.has("--quiet");

// 生成物 / 外部数据源 / 依赖：按目录名匹配（任意层级）
const EXCLUDED_DIRS = new Set([
  ".git",
  "node_modules",
  "target",
  "dist",
  "build",
  "coverage",
  "output",
  ".npm-cache",
  ".workbuddy",
  ".codeartsdoer",
  "knowledge-sources",
]);

// 二进制：按 .gitattributes 的 binary 段
const BINARY_EXT = new Set([
  ".png", ".jpg", ".jpeg", ".gif", ".ico", ".icns", ".bmp", ".webp", ".avif",
  ".pdf", ".woff", ".woff2", ".ttf", ".otf", ".eot",
  ".zip", ".gz", ".tar", ".xz", ".7z", ".rar",
  ".db", ".sqlite", ".sqlite3",
  ".wasm", ".so", ".dylib", ".dll", ".exe", ".msi", ".dmg", ".deb", ".rpm",
  ".apk", ".aab", ".pdb", ".class", ".pyc", ".o", ".a", ".lib", ".node", ".bin",
]);

// 必须 CRLF（Windows 脚本）
const CRLF_REQUIRED_EXT = new Set([".bat", ".cmd", ".ps1"]);

// 受检文本类型（未列出的扩展名一律跳过，避免误判）
const TEXT_EXT = new Set([
  ".rs", ".toml", ".lock",
  ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".mts", ".cts", ".d.ts",
  ".json", ".jsonc", ".json5",
  ".md", ".markdown", ".mdx",
  ".yaml", ".yml",
  ".sh", ".bash", ".zsh",
  ".css", ".scss", ".less", ".sass",
  ".html", ".htm", ".vue", ".svelte",
  ".xml", ".svg", ".sql", ".graphql", ".gql", ".proto",
  ".conf", ".cfg", ".ini", ".properties", ".env", ".txt", ".map",
  ".kt", ".java", ".swift", ".dart", ".gradle", ".rb", ".py", ".go", ".php",
  ".gitignore", ".gitattributes", ".editorconfig", ".npmrc", ".nvmrc",
  ".prettierrc", ".eslintrc", ".babelrc", ".yarnrc",
]);

// 无扩展名但属于文本的文件
const TEXT_BASENAMES = new Set([
  "Dockerfile", "Makefile", "LICENSE", "LICENCE", "README", "NOTICE",
  "Cargo.toml", "Cargo.lock", "Justfile", "Procfile", ".gitignore",
  ".gitattributes", ".editorconfig", ".env", ".dockerignore", ".prettierignore",
  ".eslintignore", ".npmrc", ".nvmrc", ".node-version",
]);

function isBinary(buf) {
  // 前 8KB 内出现 NUL 即视为二进制（对 UTF-16 文本文件会误判，本仓库不存在）
  const probe = buf.subarray(0, 8192);
  return probe.includes(0);
}

function walk(dir, out) {
  let entries;
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch {
    return;
  }
  for (const e of entries) {
    const full = join(dir, e.name);
    if (e.isDirectory()) {
      if (EXCLUDED_DIRS.has(e.name)) continue;
      walk(full, out);
      continue;
    }
    if (!e.isFile()) continue;
    out.push(full);
  }
}

function classify(relPath) {
  const base = relPath.split(sep).pop() ?? relPath;
  const ext = extname(base).toLowerCase();
  if (BINARY_EXT.has(ext)) return "binary";
  // Windows 脚本必须受检（Rule 2），否则会因不在 TEXT_EXT 白名单里被当成 skip 漏掉
  if (CRLF_REQUIRED_EXT.has(ext)) return "text";
  if (TEXT_BASENAMES.has(base)) return "text";
  if (TEXT_EXT.has(ext)) return "text";
  return "skip";
}

function countOccurrences(buf, needle) {
  let n = 0;
  let i = buf.indexOf(needle);
  while (i !== -1) {
    n += 1;
    i = buf.indexOf(needle, i + needle.length);
  }
  return n;
}

const files = [];
walk(ROOT, files);

const violations = [];
const fixed = [];
let scanned = 0;
let skippedBinary = 0;

for (const full of files) {
  const rel = relative(ROOT, full).split(sep).join("/");
  const kind = classify(rel);
  if (kind === "binary") {
    skippedBinary += 1;
    continue;
  }
  if (kind === "skip") continue;

  let buf;
  try {
    // 大文件只读头部判断类型，避免整读 11MB 级文件
    const size = statSync(full).size;
    if (size > 32 * 1024 * 1024) continue;
    buf = readFileSync(full);
  } catch {
    continue;
  }
  if (isBinary(buf)) {
    skippedBinary += 1;
    continue;
  }
  scanned += 1;

  const ext = extname(rel).toLowerCase();
  const crlfCount = countOccurrences(buf, "\r\n");
  const lfTotal = countOccurrences(buf, "\n");
  // 裸 LF = 总 LF 数 - CRLF 占用的 LF 数。
  // 不能直接用 buf.includes(0x0a) 判断是否混用：CRLF 本身含 0x0a，纯 CRLF 会被误判成混用。
  const loneLF = lfTotal - crlfCount;
  const hasCRLF = crlfCount > 0;
  const wantsCRLF = CRLF_REQUIRED_EXT.has(ext);

  if (wantsCRLF) {
    // 单行且无换行符的文件（如单行命令脚本）不存在"行尾"概念，不适用本规则 ——
    // 否则 --fix 无处可替换，会陷入"修完复查仍报"的死循环。
    const hasAnyNewline = lfTotal > 0 || buf.includes(0x0d);
    if (hasAnyNewline && !hasCRLF) {
      violations.push({
        rel,
        kind: "missing-crlf",
        detail: "Windows 脚本必须为 CRLF（cmd.exe / PowerShell 解析依赖）",
      });
      if (FIX) {
        writeFileSync(full, buf.toString("utf8").replace(/\r?\n/g, "\r\n"));
        fixed.push(rel);
      }
    }
    continue;
  }

  if (hasCRLF) {
    violations.push({
      rel,
      kind: "crlf",
      detail: loneLF > 0
        ? `混用 CRLF(${crlfCount}) 与裸 LF(${loneLF})`
        : `纯 CRLF（${crlfCount} 行）`,
    });
    if (FIX) {
      writeFileSync(full, buf.toString("utf8").replace(/\r\n/g, "\n"));
      fixed.push(rel);
    }
  }
}

const pad = (s) => String(s).padEnd(6);

if (!QUIET) {
  console.log("=== Line Endings Check ===");
  console.log(`mode: ${FIX ? "fix" : "check"}`);
  console.log(`scanned: ${scanned} text file(s), skipped: ${skippedBinary} binary file(s)`);
  console.log();
}

if (FIX) {
  console.log(`--- Fixed ${fixed.length} file(s) ---`);
  for (const f of fixed.slice(0, 50)) console.log(`  FIXED  ${f}`);
  if (fixed.length > 50) console.log(`  ... 另有 ${fixed.length - 50} 个文件已修正`);
  console.log();
  console.log("=== Summary ===");
  console.log(`All line endings normalized to ${"LF"}. Remaining: 0 violation(s).`);
  process.exit(0);
}

// 按类型分组输出
const crlfHits = violations.filter((v) => v.kind === "crlf");
const missingCRLF = violations.filter((v) => v.kind === "missing-crlf");

if (!QUIET) {
  console.log("--- Rule 1: Text files must use LF ---");
  if (crlfHits.length === 0) {
    console.log("PASS: No CRLF found");
  } else {
    console.log(`FAIL: ${crlfHits.length} file(s) contain CRLF:`);
    for (const v of crlfHits.slice(0, 50)) console.log(`  ${v.rel}  (${v.detail})`);
    if (crlfHits.length > 50) console.log(`  ... 另有 ${crlfHits.length - 50} 个文件`);
  }
  console.log();

  console.log("--- Rule 2: Windows scripts (.bat/.cmd/.ps1) must use CRLF ---");
  if (missingCRLF.length === 0) {
    console.log("PASS: No violations");
  } else {
    console.log(`FAIL: ${missingCRLF.length} file(s) should be CRLF:`);
    for (const v of missingCRLF) console.log(`  ${v.rel}  (${v.detail})`);
  }
  console.log();
}

console.log("=== Summary ===");
if (violations.length === 0) {
  console.log("All line endings check passed.");
  process.exit(0);
}

console.log(`Found ${violations.length} line ending violation(s).`);
console.log();
console.log("修复方式：");
console.log("  node scripts/check-line-endings.mjs --fix");
console.log("  （编辑器请安装 EditorConfig 插件，保存时自动写 LF —— 见 .editorconfig）");
process.exit(1);
