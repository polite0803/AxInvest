// scripts/check-i18n-key-exists.mjs
// 验证所有 AxInvest 特有的 t() 键都在 11 种 locale 文件中存在。
// 退出码 0 = 全部通过，非 0 = 有缺失键。
import { readdirSync, readFileSync } from "fs";
import { join } from "path";

const LOCALE_DIR = "src/i18n/locales";
const SOURCE_DIRS = ["src/components", "src/stores", "src/pages", "src/lib", "src/hooks"];

// ── 1. 收集 AxInvest 特有键 ──────────────────────────────────────────
const usedKeys = new Map();

function scanDir(dir, regex) {
  try {
    const entries = readdirSync(dir, { withFileTypes: true });
    for (const e of entries) {
      const fp = join(dir, e.name);
      if (e.isDirectory() && !e.name.startsWith(".") && e.name !== "node_modules") { scanDir(fp, regex); }
      else if (e.name.endsWith(".ts") || e.name.endsWith(".tsx")) {
        for (const m of readFileSync(fp, "utf8").matchAll(regex)) {
          const key = m[1];
          if (!usedKeys.has(key)) { usedKeys.set(key, new Set()); }
          usedKeys.get(key).add(fp);
        }
      }
    }
  } catch { /* dir may not exist */ }
}

// 只扫描 AxInvest 相关的 section
// common.* 段虽非 AxInvest 特有,但在 stock-analysis 等核心模块也被大量使用,
// 漏掉会导致 t("common.xxx") 显示原始 key。统一扫描避免盲点。
const AX_INVEST_SECTIONS = ["stockAnalysis", "trade", "nav", "common"];
for (const section of AX_INVEST_SECTIONS) {
  const re = new RegExp(`t\\("(${section}\\.[^"]+)"\\)`, "g");
  for (const dir of SOURCE_DIRS) { scanDir(dir, re); }
}
// i18n.t() in stores
const re2 = /i18n\.t\("(stockAnalysis\.[^"]+)"\)/g;
scanDir("src/stores", re2);

console.log(`Scanned, found ${usedKeys.size} AxInvest i18n keys`);

// ── 2. 检查每个键在 11 种 locale 中（支持嵌套路径）────────────────
const localeFiles = readdirSync(LOCALE_DIR).filter((f) => f.endsWith(".json"));
let totalMissing = 0;

function deepHas(obj, path) {
  // First try nested: a.b.c
  const parts = path.split(".");
  let cur = obj;
  let nestedOk = true;
  for (const p of parts) {
    if (!cur || typeof cur !== "object") {
      nestedOk = false;
      break;
    }
    if (!(p in cur)) {
      nestedOk = false;
      break;
    }
    cur = cur[p];
  }
  if (nestedOk) { return true; }
  // Then try flat key at top level: "a.b.c" as a single key in obj
  if (path in obj && typeof obj[path] === "string") { return true; }
  // Then try flat key: first section is obj, rest is a flat dot-key
  const dotIdx = path.indexOf(".");
  if (dotIdx === -1) { return false; }
  const sec = path.substring(0, dotIdx);
  const flatKey = path.substring(dotIdx + 1);
  return typeof obj[sec] === "object" && obj[sec] !== null && (flatKey in obj[sec]);
}

for (const [fullKey, files] of usedKeys) {
  for (const lf of localeFiles) {
    const j = JSON.parse(readFileSync(join(LOCALE_DIR, lf), "utf8"));
    if (!deepHas(j, fullKey)) {
      totalMissing++;
      const fileList = [...files].slice(0, 2).join(", ");
      console.log(`MISSING: ${lf} → ${fullKey} (${fileList})`);
    }
  }
}

if (totalMissing > 0) {
  console.log(`\n❌ ${totalMissing} missing key(s). Run: node scripts/post-merge-stock.mjs`);
  process.exit(1);
}

console.log("✅ All AxInvest i18n keys present");
process.exit(0);
