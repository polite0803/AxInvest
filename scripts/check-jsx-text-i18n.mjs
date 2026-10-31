#!/usr/bin/env node
// scripts/check-jsx-text-i18n.mjs
// JSX children 文本节点 i18n 门禁（2026-10-31，审查清单建议 #1 落地）。
//
// 补的是什么盲区：既有五道 i18n 门（i18n-scan / check_i18n.py / untranslated /
// key-parity / placeholder-usage）全部只看「属性字面量」或「locale 文件层」——
// **JSX children 里的裸文本**（`<label>Operation</label>`）一道都看不见。
// 实测负控样本：`FileOperationPropertyPanel.tsx` 的 4 条标签（Operation /
// File Path / Content / Output Var）在五道门下全绿，CI 恒绿。
//
// 判据（双向）：
//   J1  JSX children 文本含 CJK        ⇒ 违规（中文 UI 文案必须走 t()）
//   J2  JSX children 文本含英文单词(≥2 连续字母)
//       且 trim 后不等于任何 locale 的既有值 ⇒ 违规（没有翻译就没有 key，
//       它也不在 t() 里 —— 纯裸串）
//   其余（纯数字 / 标点 / 单字母 / emoji）忽略。
//
// 提取形态（v1 边界，刻意保守）：只抓「同一行内 `>文本</tag>`」——右侧必须紧跟
// 闭合标签。理由：无锚的 `>…<` 会把比较运算符（`a > b`、`Record<string,…>`）
// 成批误判成 JSX 文本（实测 174 条里绝大多数是这类假阳性）；锚定 `</tag>` 后
// 存量收敛到 51 条且全部是真实 JSX 文本节点。代价：跨行文本节点
// （`<div>\n  text\n</div>`）不在扫描面 —— v1 不抓，先守住最高频的单行形态。
//
// 基线（棘轮，只减不增）：scripts/jsx-text-i18n-baseline.json，键 = file + text
// （**不含行号** —— 行号键基线会随任何编辑成批漂移腐烂，这正是
// .i18n-allowlist.json 6032 行里大量漂移条目的教训）。`--update-baseline`
// 用当前违规集重写基线（自动剔除已修复项），重写后条目只许减少。
// 文件级 `i18n-exempt` 标记沿用 i18n-scan.mjs 同一惯例（整文件豁免）。
//
// 用法：
//   node scripts/check-jsx-text-i18n.mjs                 # 门禁（默认严格）
//   node scripts/check-jsx-text-i18n.mjs --selftest      # 内建正负对照（不碰真实 src）
//   node scripts/check-jsx-text-i18n.mjs --update-baseline
//
// 退出码：0 = 通过（或 selftest 全过）；1 = 有基线外违规 / selftest 失败。

import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = join(__dirname, "..");
const LOCALES_DIR = join(root, "src", "i18n", "locales");
const BASELINE = join(root, "scripts", "jsx-text-i18n-baseline.json");

const args = process.argv.slice(2);
const SELFTEST = args.includes("--selftest");
const UPDATE = args.includes("--update-baseline");

const CJK = /[㐀-䶿一-鿿]/;
// ≥2 连续英文字母才算「英文单词」——单字母（步骤标号 >A<、选项 >N<）与
// 纯标点/数字（>|<、>1<、>:<、>&nbsp;<）是排版符号，不是文案。
const HAS_WORD = /[A-Za-z]{2,}/;

// ── locale 值全集（11 语言并集）：文本 = 任何一份已有译文 ⇒ 视为「已存在 key」──
function flattenValues(obj, out) {
  for (const k of Object.keys(obj)) {
    const v = obj[k];
    if (typeof v === "string") {
      if (v.trim()) { out.add(v.trim()); }
    } else if (v && typeof v === "object") {
      flattenValues(v, out);
    }
  }
  return out;
}

function loadLocaleValues() {
  const set = new Set();
  if (!existsSync(LOCALES_DIR)) { return set; }
  for (const f of readdirSync(LOCALES_DIR)) {
    if (!f.endsWith(".json")) { continue; }
    try {
      flattenValues(JSON.parse(readFileSync(join(LOCALES_DIR, f), "utf8")), set);
    } catch {
      // 语法错误由 check_i18n.py 那道门负责，这里不重复报
    }
  }
  return set;
}

// ── 注释剥离（与 i18n-scan.mjs 同源：// 行内、/* */ 含 JSX {/* */}、/** */）──
function stripComments(lines) {
  const result = [];
  let inBlock = false;
  for (const raw of lines) {
    let s = raw;
    if (inBlock) {
      const end = s.indexOf("*/");
      if (end === -1) {
        result.push("");
        continue;
      }
      s = s.slice(end + 2);
      inBlock = false;
    }
    let guard = 0;
    while (true) {
      const start = s.indexOf("/*");
      if (start === -1) { break; }
      const end = s.indexOf("*/", start + 2);
      if (end === -1) {
        s = s.slice(0, start);
        inBlock = true;
        break;
      }
      s = s.slice(0, start) + s.slice(end + 2);
      if (++guard > 50) { break; }
    }
    const m = s.match(/(^|[^:])\/\//);
    if (m) { s = s.slice(0, m.index + (m[1] ? m[1].length : 0)); }
    result.push(s);
  }
  return result;
}

function normalizePath(p) {
  let s = p.replace(/\\/g, "/");
  if (s.startsWith("./")) { s = s.slice(2); }
  return s;
}

function collectTsx(dir, out) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    const st = statSync(p);
    if (st.isDirectory()) {
      if (name === "locales" && dir.replace(/\\/g, "/").endsWith("i18n")) { continue; }
      if (name === "__tests__") { continue; }
      collectTsx(p, out);
    } else if (/\.tsx$/.test(name) && !/\.test\.|\.spec\./.test(name)) {
      out.push(p);
    }
  }
  return out;
}

// ── 单文件扫描：返回 {rule, file, line, text}[] ──
// 右侧锚 `</tag>`：`>文本</label>`。捕获组 1 = 文本节点。
const JSX_TEXT = />([^<>{}\n]+?)<\/\s*[A-Za-z][\w.-]*\s*>/g;

export function scanTsxContent(content, rel) {
  if (content.includes("i18n-exempt")) { return []; }
  const cleaned = stripComments(content.split("\n")).join("\n");
  const out = [];
  const lines = cleaned.split("\n");
  lines.forEach((line, idx) => {
    JSX_TEXT.lastIndex = 0;
    let m;
    while ((m = JSX_TEXT.exec(line)) !== null) {
      const text = m[1].trim();
      if (!text) { continue; }
      if (CJK.test(text)) {
        out.push({ rule: "J1", file: rel, line: idx + 1, text });
        continue;
      }
      if (HAS_WORD.test(text)) {
        // locale 值命中检查由调用方注入（纯函数化，selftest 不依赖真实 locale）
        out.push({ rule: "J2?", file: rel, line: idx + 1, text });
      }
    }
  });
  return out;
}

/** 对 J2? 候选做 locale 值裁决：命中 ⇒ 放行；未命中 ⇒ J2 违规。 */
export function adjudicate(candidates, localeValues) {
  return candidates.map((c) =>
    c.rule === "J2?" && localeValues.has(c.text) ? null : { ...c, rule: c.rule === "J2?" ? "J2" : c.rule },
  ).filter(Boolean);
}

// ── selftest：合成夹具正负对照（不碰真实 src、不依赖 locale 文件）──
function runSelftest() {
  const fixture = [
    '<div>',
    '  <label style={{ fontSize: 12 }}>Operation</label>',
    '  <label style={{ fontSize: 12 }}>File Path</label>',
    '  <span>Content</span>',
    '  <span>Output Var</span>',
    '  <span>{t("workflow.fileOperation.operation")}</span>',
    '  <span>{count > 3 ? "many" : "few"}</span>',
    '  <span>1</span>',
    '  <span>|</span>',
    '  <span>{`Total: ${n}`}</span>',
    '  {/* 注释里的 >中文< 不算 */}',
    '  <span>已保存</span>',
    '</div>',
  ].join("\n");
  const locales = new Set(["读取", "写入", "已翻译值"]);
  const candidates = scanTsxContent(fixture, "fixture.tsx");
  const viols = adjudicate(candidates, locales);
  const texts = viols.map((v) => `${v.rule}:${v.text}`);

  const mustFail = ["J2:Operation", "J2:File Path", "J2:Content", "J2:Output Var", "J1:已保存"];
  const mustPass = ["已翻译值", "Total: ${n}", "many", "1", "|"];
  const misses = mustFail.filter((k) => !texts.includes(k));
  const falsePositives = viols.filter((v) => mustPass.some((p) => v.text.includes(p.replace("${n}", "")) && v.rule === "J2"));

  if (misses.length > 0) {
    console.error(`SELFTEST FAIL：已知缺陷形态未转红：${misses.join(", ")}`);
    console.error(`  实际产出：${texts.join(" | ")}`);
    return false;
  }
  if (falsePositives.length > 0) {
    console.error(`SELFTEST FAIL：正控被误判：${falsePositives.map((v) => v.text).join(", ")}`);
    return false;
  }
  console.log(`SELFTEST OK：${mustFail.length} 条负控全部转红，正控（t() 表达式 / 表达式容器 / 纯数字 / 标点）全部放行`);
  return true;
}

// ── 基线 ──
function loadBaseline() {
  try {
    const b = JSON.parse(readFileSync(BASELINE, "utf8"));
    return { entries: b.entries || [], version: b.version || "1" };
  } catch {
    return { entries: [], version: "1" };
  }
}

// ── main ──
if (SELFTEST) {
  process.exit(runSelftest() ? 0 : 1);
}

const localeValues = loadLocaleValues();
const files = collectTsx(join(root, "src"), []);
const all = [];
for (const f of files) {
  let content;
  try {
    content = readFileSync(f, "utf8");
  } catch {
    continue;
  }
  const rel = normalizePath(f).replace(/^.*?\/src\//, "src/");
  const candidates = scanTsxContent(content, rel);
  all.push(...adjudicate(candidates, localeValues));
}

const baseline = loadBaseline();
const baselineKeys = new Set(baseline.entries.map((e) => `${e.file}\u0000${e.text}`));

if (UPDATE) {
  const seen = new Map();
  for (const v of all) {
    const key = `${v.file}\u0000${v.text}`;
    if (!seen.has(key)) { seen.set(key, { file: v.file, text: v.text }); }
  }
  const prev = new Map(baseline.entries.map((e) => [`${e.file}\u0000${e.text}`, e]));
  const entries = [];
  for (const [key, e] of seen) {
    entries.push({ file: e.file, text: e.text, reason: prev.get(key)?.reason || "存量裸文本（待翻译或技术串）" });
  }
  entries.sort((a, b) => a.file.localeCompare(b.file) || a.text.localeCompare(b.text));
  const doc = {
    version: "1",
    updated: new Date().toISOString().slice(0, 10),
    note: "JSX children 裸文本基线（棘轮：键 = file+text，只许减少）。行号刻意不进键 —— 行号键会随编辑成批漂移（.i18n-allowlist.json 教训）。",
    total: entries.length,
    entries,
  };
  writeFileSync(BASELINE, JSON.stringify(doc, null, 2) + "\n");
  const removed = baseline.entries.length - entries.length;
  console.log(`Baseline updated: ${entries.length} entries (${removed >= 0 ? `-${removed}` : `+${-removed}`} vs previous).`);
  if (entries.length > baseline.entries.length && baseline.entries.length > 0) {
    console.error("⚠ 基线变大了 —— 棘轮只许减少。先修代码，再更新基线。");
    process.exit(1);
  }
  process.exit(0);
}

const fresh = all.filter((v) => !baselineKeys.has(`${v.file}\u0000${v.text}`));

console.log(`\n=== JSX children 文本节点 i18n 检查 ===`);
console.log(`扫描 ${files.length} 个 .tsx；文本节点命中（未走 t() 且非 locale 值）：全部 ${all.length}，基线外 ${fresh.length}`);

if (fresh.length > 0) {
  console.log(`\nFAIL: ${fresh.length} 处基线外裸文本：`);
  for (const v of fresh.slice(0, 50)) {
    console.log(`  ${v.rule} ${v.file}:${v.line}: ${JSON.stringify(v.text)}`);
  }
  if (fresh.length > 50) { console.log(`  ... 还有 ${fresh.length - 50} 处`); }
  console.log(`\n修法：文案走 t() 并补 11 语言 key；确属技术串/品牌名/license id 等非 UI 文案，`);
  console.log(`先人审确认后 node scripts/check-jsx-text-i18n.mjs --update-baseline（棘轮只许减少）。`);
  process.exit(1);
}

console.log("PASS: 基线外零违规。");
process.exit(0);
