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
const AX_INVEST_SECTIONS = ["stockAnalysis", "trade", "nav"];
for (const section of AX_INVEST_SECTIONS) {
  const re = new RegExp(`t\\("(${section}\\.[^"]+)"\\)`, "g");
  for (const dir of SOURCE_DIRS) { scanDir(dir, re); }
}
// i18n.t() in stores
const re2 = /i18n\.t\("(stockAnalysis\.[^"]+)"\)/g;
scanDir("src/stores", re2);

console.log(`Scanned, found ${usedKeys.size} AxInvest i18n keys`);

// ── 2. 检查每个键在 11 种 locale 中 ─────────────────────────────────
const localeFiles = readdirSync(LOCALE_DIR).filter((f) => f.endsWith(".json"));
let totalMissing = 0;

for (const [fullKey, files] of usedKeys) {
  const dotIdx = fullKey.indexOf(".");
  const section = fullKey.substring(0, dotIdx);
  const subKey = fullKey.substring(dotIdx + 1);

  for (const lf of localeFiles) {
    const j = JSON.parse(readFileSync(join(LOCALE_DIR, lf), "utf8"));
    const sec = j[section];
    if (!sec || !(subKey in sec)) {
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
