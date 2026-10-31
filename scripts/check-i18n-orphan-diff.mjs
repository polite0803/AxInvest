#!/usr/bin/env node
// scripts/check-i18n-orphan-diff.mjs
// 增量孤儿 i18n key 门禁（2026-10-31，审查清单建议 #4 落地，code-reviewer 终稿）。
//
// 补的是什么盲区：删除页面/组件后遗留的 i18n key，**现有五道 i18n 门一道都不红**——
//   `check_i18n.py:206-209` 孤儿只报告、`:212` 退出条件不含 orphan；实测孤儿
//   7269 / 17726 = 41.0%，脚本仍输出「🎉 全部通过」。
//
// 算法（5 步，判据与 `check_i18n.py` **逐字同源**，不另写一套）：
//   输入  git diff --name-status HEAD（默认；pre-commit 可用、不联网）
//   1. K = 现存 ts/tsx/js/jsx 的静态 t() key 全集
//        （T_CALL_RE / is_real_i18n_key / NOISE_TOP 与 check_i18n.py 逐字同源；
//         排除 src/i18n/ 工具文件 —— compare_locales.js 等）
//   2. P = 运行时扫描现存代码 t(`X${` 的动态前缀 X   ← 刻意**不**硬编码 top-N
//   3. U = Σ 被删/改名/修改文件旧内容（git show HEAD:<old>）中的静态 t() key
//   4. 候选 = U − K − {k|∃p∈P: k.startsWith(p)} − {k|k.startsWith("error.")}
//   5. 候选非空 ⇒ exit 1（要求同步删除 11 语言 key）
//
// ⚠ 为什么 P 必须运行时抽取（**不许被「优化」回静态清单**）：
//   实测 t(`前缀${…}`) 共 219 处 / 113 个不同前缀，只出现 1 次的前缀 78 个。
//   硬编码 top-10 只覆盖 87/219 = 39.7%，剩余 103 个长尾前缀全部转假阳性——
//   与「守卫只守它被写下的那一刻已存在的门」（ci-check.mjs:347-351 存案）同一失败模式。
//
// ⚠ 为什么 `error.*` 整命名空间豁免：后端错误码按 `error.${code}` 拼接
//   （capabilityStore.ts / deviceSyncStore.ts / AdvancedSettings.tsx 三处实证）。
//
// R/M/D 统一走同一套减集（不写 rename 特判分支）：R 的旧内容取 `R100\told\tnew`
//   的**第二列**（tab 分隔 3 列，别按空格 split）；「现存引用全集」K 扫的是
//   **变更后工作区** ⇒ 改名/重写后仍被引用的 key 自动留在 K、自动从候选减掉。
//
// 熔断（防大变更/历史包袱把信号淹死）——触发即**显式**降级为仅报告：
//   · 本次变更 src/** 文件数 > 50（常态删改是 1-5 个文件，p90 单文件 key 数 38 ⇒ 50 是整批兜底）
//   · 或候选 key 数 > 200（≈5 个 p90 文件的量级）
//   降级打印「⚠ 已降级为仅报告，跳过 ≠ 通过」，沿用 ci-check.mjs 既有惯例；禁止静默。
//   阈值可用 --max-files / --max-candidates 覆盖（写在头部的数不许被静默改掉）。
//
// 取不到 git（不在仓库内 / git 不可用）⇒ 显式打印「跳过 ≠ 通过」，exit 0，不静默
//   —— 静默略过会让本地绿冒充 CI 绿。
//
// 用法：
//   node scripts/check-i18n-orphan-diff.mjs                     # 门禁
//   node scripts/check-i18n-orphan-diff.mjs --selftest          # 内建正负对照（不碰 git / 真实 src）
//   node scripts/check-i18n-orphan-diff.mjs --max-files 80 --max-candidates 300
//
// 退出码：0 = 无候选（或 selftest 全过 / 显式跳过）；1 = 有候选（须删 key）/ selftest 失败。

import { execSync } from "node:child_process";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = join(__dirname, "..");
const SRC_DIR = join(root, "src");

const args = process.argv.slice(2);
const SELFTEST = args.includes("--selftest");

// 熔断阈值（可被参数覆盖；默认值来自 2026-10-31 实测，见头部注释）
function numArg(name, dflt) {
  const i = args.indexOf(name);
  if (i < 0 || !args[i + 1]) { return dflt; }
  const n = Number(args[i + 1]);
  return Number.isFinite(n) && n > 0 ? n : dflt;
}
const MAX_FILES = numArg("--max-files", 50);
const MAX_CANDIDATES = numArg("--max-candidates", 200);

// ── 与 check_i18n.py 逐字同源的三件套（改动任一侧必须同步另一侧）──
// check_i18n.py:39-48 T_CALL_RE（Python VERBOSE 正则的 JS 等价形态）
const T_CALL_RE = /t\(\s*(?:["']([a-zA-Z_][\w.]*)["']|`([a-zA-Z_][\w.]*)`)[,\s)]/g;
// check_i18n.py:51-58 NOISE_TOP
const NOISE_TOP = new Set([
  "antd", "common", "api", "a", "import", "function", "handler",
  "html2canvas", "iframe", "navigate", "pre", "prompt", "schema",
  "script", "template", "textarea", "timelineJump", "trigger",
  "vendor_", "view", "wikiId", "left", "right", "json",
  "analysisId", "import_", "toolchain", "layout", "stockCode", "tab",
  "evolutionConsentModal",
]);

/** check_i18n.py:73-88 is_real_i18n_key 的 JS 逐字同源实现。 */
export function isRealI18nKey(rawKey) {
  const key = rawKey.trim();
  // 1) 必须至少两段（"foo.bar"），单段的 99% 是技术名词
  if (!key.includes(".")) { return false; }
  // 2) 字母/下划线开头，只能含字母数字下划线和点
  if (!/^[a-zA-Z][a-zA-Z0-9_]*(?:\.[a-zA-Z][a-zA-Z0-9_]*)*$/.test(key)) { return false; }
  // 3) 顶部前缀不在噪声集合里
  if (NOISE_TOP.has(key.split(".")[0])) { return false; }
  // 4) 点号结尾的截断 key（正则误匹配）
  if (key.endsWith(".")) { return false; }
  return true;
}

/** 从一段源码文本抽取静态 t() key 集合（K 与 U 共用同一实现）。 */
export function extractKeysFromContent(content) {
  const keys = new Set();
  let m;
  T_CALL_RE.lastIndex = 0;
  while ((m = T_CALL_RE.exec(content)) !== null) {
    const key = m[1] ?? m[2];
    if (key && isRealI18nKey(key)) { keys.add(key); }
  }
  return keys;
}

/** 运行时抽取动态前缀：t(`X${` 的 X（不解析模板串内容）。
 * X 含源码字面的尾点：t(`ns.${x}`) ⇒ "ns."（[\w.] 贪婪吃掉 `${` 前的点）——
 * 这与该模板的运行时键空间**精确对齐**，startsWith 不会误豁免 nsXxx 近名命名空间。
 */
const DYN_PREFIX_RE = /t\(\s*`([A-Za-z_][\w.]*)\$\{/g;
export function extractDynamicPrefixes(content) {
  const out = new Set();
  let m;
  DYN_PREFIX_RE.lastIndex = 0;
  while ((m = DYN_PREFIX_RE.exec(content)) !== null) {
    out.add(m[1]);
  }
  return out;
}

/**
 * 候选 = U − K − {k|∃p∈P: k.startsWith(p)} − {k|k.startsWith("error.")}
 * 纯函数：selftest 与正式跑共用。
 */
export function computeCandidates({ uKeys, kKeys, prefixes }) {
  const prefixList = [...prefixes];
  return [...uKeys]
    .filter((k) => !kKeys.has(k))
    .filter((k) => !prefixList.some((p) => k.startsWith(p)))
    .filter((k) => !k.startsWith("error."))
    .sort();
}

/** 解析 `git diff --name-status HEAD` 输出。R 行是 tab 分隔 3 列（R100\told\tnew）。 */
export function parseNameStatus(text) {
  const entries = [];
  for (const line of text.split("\n")) {
    const t = line.trim();
    if (!t) { continue; }
    const cols = t.split("\t");
    const status = cols[0][0]; // R100 → R
    if (status === "R" && cols.length >= 3) {
      entries.push({ status, oldPath: cols[1], newPath: cols[2] });
    } else if (status === "M" || status === "D") {
      entries.push({ status, oldPath: cols[1], newPath: cols[1] });
    }
    // A：新增文件没有旧内容，不产候选
  }
  return entries;
}

// ── 工作区文件收集（与 check_i18n.py scan_code_t_keys 同口径：ts/tsx/js/jsx，跳 .d.ts）──
function collectSrcFiles(dir, out) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    const st = statSync(p);
    if (st.isDirectory()) {
      // 排除 i18n 工具文件（compare_locales.js 等）与 locale JSON（本来就不在扩展名集）
      if (name === "i18n") { continue; }
      collectSrcFiles(p, out);
    } else if (/\.(ts|tsx|js|jsx)$/.test(name) && !name.endsWith(".d.ts")) {
      out.push(p);
    }
  }
  return out;
}

function readSafe(p) {
  try {
    return readFileSync(p, "utf8");
  } catch {
    return "";
  }
}

function git(argsStr) {
  // maxBuffer 必须显式放大：默认 1MB，实测 hi.json 1.26MB / ru.json 1.11MB 直接 ENOBUFS，
  // 会被误报成「旧内容取不到」（2026-10-31 实测）。
  return execSync(`git ${argsStr}`, {
    cwd: root,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    maxBuffer: 64 * 1024 * 1024,
  });
}

// ── selftest：合成夹具正负对照（不碰 git、不碰真实 src）──
function runSelftest() {
  let fail = 0;
  const check = (cond, msg) => {
    if (!cond) {
      console.error(`SELFTEST FAIL：${msg}`);
      fail++;
    }
  };

  // 1) name-status 解析：M/D/R（3 列）/A 跳过
  const entries = parseNameStatus("M\tsrc/a.tsx\nD\tsrc/b.tsx\nR100\tsrc/c.tsx\tsrc/d.tsx\nA\tsrc/e.tsx\n");
  check(entries.length === 3, `name-status 应解析出 3 条（A 跳过），实际 ${entries.length}`);
  check(entries[0].oldPath === "src/a.tsx", "M 行取第 1 路径");
  check(entries[1].oldPath === "src/b.tsx" && entries[1].status === "D", "D 行取第 1 路径");
  check(entries[2].oldPath === "src/c.tsx" && entries[2].newPath === "src/d.tsx", "R 行必须取 tab 分隔的第 2 列作旧路径（按空格 split 会错）");

  // 2) K 抽取：三种引号 + 噪声过滤 + 单段拒绝
  // ⚠ 夹具里的 `$` 一律用 \u0024 写 —— 直接写 ${…} 会被 JS 在源码级求值，夹具就变味了
  //   （初版 selftest 三连 FAIL 的真根因，不是正则错）。
  // ⚠ 双引号样本用 alpha.beta 而非 a.b：NOISE_TOP 有顶段 "a"（与 check_i18n.py 同源），
  //   a.b 被噪声过滤是**正常行为**，于是顺手把它留作噪声负控样本。
  const D = String.fromCharCode(36); // "$"
  const k = extractKeysFromContent(
    't("alpha.beta"); t(\'c.d\'); t(`e.f`); t("antd.x"); t("a.b"); t("single"); t("error.CODE_X"); t(`stockAnalysis.timeHorizon.' + D + '{tier}`)',
  );
  check(k.has("alpha.beta") && k.has("c.d") && k.has("e.f"), "K 必须收三种引号形态");
  check(!k.has("a.b") && !k.has("antd.x"), "NOISE_TOP 顶部前缀（a/antd）必须过滤（与 check_i18n.py 同源）");
  check(!k.has("single"), "单段 key 必须拒绝");
  check(!k.has("stockAnalysis.timeHorizon"), "带插值的模板串不得产出静态 key（T_CALL_RE 语义：尾随 ${…} 非 [,\\s)]）");

  // 3) 动态前缀抽取
  // ⚠ 前缀含源码字面的尾点（[\w.] 贪婪）：t(`ns.${x}`) ⇒ "ns." —— 与运行时键空间精确对齐，
  //   startsWith 不会误豁免 nsXxx 这类近名命名空间（修正期曾误按无点口径断言而 FAIL）。
  const p = extractDynamicPrefixes(
    'const x = t(`stockAnalysis.timeHorizon.' + D + '{tier}`); const y = t(`error.' + D + '{code}`);',
  );
  check(p.has("stockAnalysis.timeHorizon.") && p.has("error."), "P 必须运行时抽出 t(`X" + D + "{` 的 X（含尾点）");

  // 4) 负控：删引用留 key ⇒ 必须出候选（门禁的核心判据）
  const cand = computeCandidates({
    uKeys: new Set(["oldPage.title", "oldPage.desc"]),
    kKeys: new Set(["other.key"]),
    prefixes: new Set(),
  });
  check(cand.includes("oldPage.title") && cand.includes("oldPage.desc"), "负控：删引用留 key 必须红");

  // 5) 正控：动态前缀覆盖 ⇒ 不红（前缀口径 = 带尾点，与 #3 一致）
  const cand2 = computeCandidates({
    uKeys: new Set(["stockAnalysis.timeHorizon.short"]),
    kKeys: new Set(),
    prefixes: new Set(["stockAnalysis.timeHorizon."]),
  });
  check(cand2.length === 0, "正控：动态前缀覆盖的 key 不得假阳性");

  // 6) 正控：error.* 豁免 ⇒ 不红
  const cand3 = computeCandidates({ uKeys: new Set(["error.SOME_CODE"]), kKeys: new Set(), prefixes: new Set() });
  check(cand3.length === 0, "正控：error.* 命名空间豁免");

  // 7) 正控：改名后仍被引用 ⇒ 留在 K、自动减掉
  const cand4 = computeCandidates({
    uKeys: new Set(["moved.key"]),
    kKeys: new Set(["moved.key"]),
    prefixes: new Set(),
  });
  check(cand4.length === 0, "正控：改名后仍被引用的 key 不出候选");

  if (fail > 0) { return false; }
  console.log("SELFTEST OK：name-status 3 列解析 / K 三引号 + 噪声 / 动态前缀运行时抽取 / 负控红 / 前缀与 error.* 豁免正控 全部通过");
  return true;
}

// ── main ──
if (SELFTEST) {
  process.exit(runSelftest() ? 0 : 1);
}

// git 可用性：显式「跳过 ≠ 通过」，不静默
let inside = false;
try {
  inside = git("rev-parse --is-inside-work-tree").trim() === "true";
} catch {
  inside = false;
}
if (!inside) {
  console.log("⚠ 取不到 git（不在仓库工作树内或 git 不可用）—— 跳过 ≠ 通过，本轮未验证增量孤儿 key。");
  process.exit(0);
}

let diffText = "";
try {
  diffText = git("diff --name-status HEAD");
} catch (e) {
  console.log(`⚠ git diff --name-status HEAD 失败：${String(e).split("\n")[0]} —— 跳过 ≠ 通过，本轮未验证。`);
  process.exit(0);
}

const entries = parseNameStatus(diffText);
const srcEntries = entries.filter((e) => e.newPath.startsWith("src/") || e.oldPath.startsWith("src/"));

console.log(`\n=== 增量孤儿 i18n key 检查（对 HEAD）===`);
console.log(`变更条目 ${entries.length}（src/** 内 ${srcEntries.length}）`);

if (srcEntries.length === 0) {
  console.log("PASS: 无 src/** 变更，无候选可言。");
  process.exit(0);
}

// 熔断 1：变更面过大 ⇒ 降级 report-only（显式打印，跳过 ≠ 通过）
if (srcEntries.length > MAX_FILES) {
  console.log(`⚠ 已降级为仅报告（变更 src/** 文件数 ${srcEntries.length} > ${MAX_FILES}），跳过 ≠ 通过 —— 请人工复核或拆小变更。`);
  process.exit(0);
}

// K 与 P：现存工作区（变更后状态）
const kKeys = new Set();
const prefixes = new Set();
for (const f of collectSrcFiles(SRC_DIR, [])) {
  const content = readSafe(f);
  for (const k of extractKeysFromContent(content)) { kKeys.add(k); }
  for (const p of extractDynamicPrefixes(content)) { prefixes.add(p); }
}

// U：被删/改名/修改文件的旧内容（只扫代码文件 —— 与 K 侧 collectSrcFiles 同口径：
//   locale JSON 没有 t() 调用，扫它是纯噪声 + 大文件 maxBuffer 风险源，实测 hi.json 1.26MB）
const isCodePath = (p) => /\.(ts|tsx|js|jsx)$/.test(p) && !p.endsWith(".d.ts");
const uKeys = new Set();
const skippedOld = [];
for (const e of srcEntries) {
  if (!isCodePath(e.oldPath)) { continue; }
  let old = "";
  try {
    old = git(`show HEAD:${e.oldPath}`);
  } catch {
    skippedOld.push(e.oldPath);
    continue;
  }
  for (const k of extractKeysFromContent(old)) { uKeys.add(k); }
}
if (skippedOld.length > 0) {
  console.log(`⚠ ${skippedOld.length} 个文件的旧内容取不到（HEAD 中不存在），已跳过并点名列出（不静默）：`);
  for (const f of skippedOld.slice(0, 10)) { console.log(`  - ${f}`); }
}

const candidates = computeCandidates({ uKeys, kKeys, prefixes });

console.log(`K（现存静态引用）${kKeys.size} · U（旧内容静态 key）${uKeys.size} · 动态前缀 ${prefixes.size}`);

// 熔断 2：候选过多 ⇒ 降级 report-only
if (candidates.length > MAX_CANDIDATES) {
  console.log(`⚠ 已降级为仅报告（候选 key 数 ${candidates.length} > ${MAX_CANDIDATES}），跳过 ≠ 通过 —— 请人工复核。`);
  for (const k of candidates.slice(0, 20)) { console.log(`  ? ${k}`); }
  process.exit(0);
}

if (candidates.length > 0) {
  console.log(`\nFAIL: ${candidates.length} 个 key 的最后引用随本次变更消失（须同步删除 11 语言 locale）：`);
  for (const k of candidates) {
    console.log(`  - ${k}`);
  }
  console.log(`\n说明：删除页面/组件后遗留的 locale key 此前五道 i18n 门全都不红（check_i18n.py 孤儿只报告）。`);
  console.log(`若属误报（动态拼接引用），把真实引用形态报给门禁维护者，不要静默扩豁免。`);
  process.exit(1);
}

console.log("PASS: 无孤儿候选。");
process.exit(0);
