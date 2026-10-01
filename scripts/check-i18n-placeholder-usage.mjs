#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
//
// i18n **占位符使用**门禁：资源里的 `{{x}}` 必须在调用点被插值，值也不得回吐键路径。
//
// ## 为什么需要它（2026-10-01）
//
// 现有四道 i18n 门禁**都看不见**这一类缺陷：
//   · `check-hardcoded-i18n.sh`   —— 只查源码里的硬编码文案；
//   · `check_i18n.py`             —— JSON 语法 / zh-CN **空值** / `t()` 引用 ↔ zh-CN **键存在性**；
//   · `check-i18n-untranslated.mjs` —— 非 CJK locale 的**值**里是否残留中文；
//   · `check-i18n-key-parity.mjs` —— 11 语言的 **key 路径集合**是否一致。
// 四道的判据都停在「键在不在、值空不空、值是不是中文」，**没有任何一条把「资源值」和
// 「调用点」对上**。于是两种形态能静默上线，用户在界面上直接看到模板原文：
//
//   ① 值回吐键路径：`"stockAnalysis.dailyReview.empty": "stockAnalysis.dailyReview.empty"`
//      —— 批量补译脚本把**键本身**当译文写进了 11 语言副本（zh-TW 一次 10 条）。
//      `check_i18n.py` 判「非空」通过，parity 判「键存在」通过。
//   ② 占位符漏插值：资源是 `应用选中项 ({{count}})`，调用点写成
//      `t("…calibrateApplySelected") + " (" + n + ")"` —— 括号里渲染成字面量
//      `({{COUNT}})`（21th 主题预设给 `.ant-btn` 加了 `text-transform: uppercase`，
//      所以截图里是大写，磁盘上其实一直是小写 `{{count}}`）。
//      同类还有 `.replace("{{n}}", …)` 但资源占位符叫 `{{count}}` —— 替换静默不命中。
//
// ## 判据
//
//   R1  任一 locale 的叶子值 == 自身完整点分路径 ⇒ 硬失败。
//   R2  zh-CN 叶子值含 `{{x}}`，且源码里存在 `t("该路径")`（右括号紧跟、无第二参、
//       也没有 `.replace("{{x}}"` 手工兜住该占位符）⇒ 硬失败。
//   R3  非 zh-CN locale 的叶子值 == 自身键名，且该键**被代码字面量引用**，且 zh-CN 同键
//       是含汉字的真译文 ⇒ 硬失败。这是「批量补译脚本把键名当译文写进去」的另一种形态
//       （`stockAnalysis.tab.market` 在 ko 里就渲染成 `market`）。
//       **只判被引用的键**：零引用的同形值不是用户可见缺陷，不拦。
//   R4  豁免条目已无命中 ⇒ 硬失败（清单只增不减 = 判据面被写窄）。
//
// R1/R2 零基线。R3 的合法例外走 `scripts/i18n-placeholder-usage-allowlist.json` 的
// `residue-exceptions`：非 CJK 语言里 `PascalCase`、`copies`、`tokens` 这类标签的**正确写法
// 就与键名同形**，无法用机械判据与残留区分，故逐条登记 locale + key + reason（现 54 条）。
// R2 的例外走同文件的 `exceptions`。两类豁免都**不写行号**（行号会腐烂）。
//
// R2 只以 zh-CN 为权威（与 `check_i18n.py` 同口径）；11 语言的插值参数名一致，
// 由本文件跑一次即覆盖 —— 换语言不会漏判，因为缺的是**调用点**不是**值**。
//
// ## 用法
//
//   node scripts/check-i18n-placeholder-usage.mjs            # 门禁
//   node scripts/check-i18n-placeholder-usage.mjs --selftest # 正控：三条判据都必须能报红
//   node scripts/check-i18n-placeholder-usage.mjs --list=20  # 每条规则最多列几条
//
// 退出码：0 = 干净；1 = 存在违规或正控失效。

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const LOCALE_DIR = path.join(ROOT, "src", "i18n", "locales");
const SRC_DIR = path.join(ROOT, "src");
const REF_LOCALE = "zh-CN.json";
const ALLOWLIST_FILE = path.join(ROOT, "scripts", "i18n-placeholder-usage-allowlist.json");
/** 源码扫描时跳过的目录名（测试夹具里 `t("key")` 无插值是合法的）。 */
const SKIP_DIRS = new Set(["node_modules", "__tests__", "assets"]);

function argOf(name, dflt) {
  const prefix = `--${name}=`;
  const hit = process.argv.find((a) => a.startsWith(prefix));
  return hit ? hit.slice(prefix.length) : dflt;
}

/** 递归展开叶子路径（`.` 连接）；数组整体算一个值。 */
function leafPaths(node, prefix, out) {
  if (node !== null && typeof node === "object" && !Array.isArray(node)) {
    for (const [k, v] of Object.entries(node)) leafPaths(v, prefix ? `${prefix}.${k}` : k, out);
  } else {
    out.set(prefix, node);
  }
  return out;
}

const PLACEHOLDER = /\{\{\s*([A-Za-z0-9_]+)\s*\}\}/g;

/** 取一个值里出现的占位符名集合。 */
function placeholdersOf(value) {
  const set = new Set();
  if (typeof value !== "string") return set;
  for (const m of value.matchAll(PLACEHOLDER)) set.add(m[1]);
  return set;
}

/** 收集 R1：值 == 自身完整点分路径。 */
function findPathEchoLeaves(dicts) {
  const hits = [];
  for (const [file, leaves] of dicts) {
    for (const [p, v] of leaves) {
      if (typeof v === "string" && v === p) hits.push(`${file}: ${p}`);
    }
  }
  return hits;
}

function* walkSource(dir) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    if (e.name.startsWith(".")) continue;
    const p = path.join(dir, e.name);
    if (e.isDirectory()) {
      if (!SKIP_DIRS.has(e.name)) yield* walkSource(p);
    } else if (/\.(?:ts|tsx)$/.test(e.name)) {
      yield p;
    }
  }
}

/**
 * 收集 R2：资源含占位符、调用点却没插值。
 *
 * 识别 `t("key"` / `i18n.t("key"`，看紧跟的定界符：
 *   `,`  ⇒ 传了第二参，判通过（参数名对不对不在本门职责内，运行时会原样留 `{{x}}`，
 *          那属 R2 的下一层，需要类型级校验才能覆盖）；
 *   `)`  ⇒ 无插值参数。此时再给 3 行窗口找 `.replace("{{x}}"` 手工兜底，
 *          兜住了该占位符才算通过（兜不住 = 名字写错，正是 PlanHistoryPanel 那处）。
 */
function findUninterpolatedCalls(refLeaves, srcFiles) {
  const phByPath = new Map();
  for (const [p, v] of refLeaves) {
    const ph = placeholdersOf(v);
    if (ph.size > 0) phByPath.set(p, ph);
  }
  const CALL = /(?:^|[^A-Za-z0-9_$.])(?:i18n\.)?t\(\s*"([^"]+)"\s*([,)])/g;
  const hits = [];
  for (const file of srcFiles) {
    const lines = fs.readFileSync(file, "utf8").split(/\r?\n/);
    lines.forEach((line, i) => {
      const trimmed = line.trimStart();
      if (trimmed.startsWith("//") || trimmed.startsWith("*") || trimmed.startsWith("/*")) return;
      CALL.lastIndex = 0;
      let m;
      while ((m = CALL.exec(line)) !== null) {
        const [, key, sep] = m;
        const ph = phByPath.get(key);
        if (!ph) continue;
        if (sep === ",") continue;
        const window = lines.slice(i, i + 4).join("\n");
        const stillMissing = [...ph].filter((name) => !window.includes(`"{{${name}}}"`) && !window.includes(`\`{{${name}}}\``));
        if (stillMissing.length === 0) continue;
        hits.push({
          file: path.relative(ROOT, file).split(path.sep).join("/"),
          key,
          line: i + 1,
          text: `${path.relative(ROOT, file)}:${i + 1}  t("${key}") 未插值 {${stillMissing.join(",")}}  值: ${refLeaves.get(key)}`,
        });
      }
    });
  }
  return hits;
}

/**
 * 按「文件 + 键」放行 R2 命中，并反查失效条目。
 * 失效（豁免还在、命中已消失）必须报红 —— 否则清单只增不减，判据面被静默写窄。
 */
function applyAllowlist(hits, allowlist) {
  const kept = [];
  const used = new Set();
  for (const h of hits) {
    const idx = allowlist.findIndex((e) => e.file === h.file && e.key === h.key);
    if (idx < 0) {
      kept.push(h);
      continue;
    }
    used.add(idx);
  }
  return { kept, stale: allowlist.filter((_, i) => !used.has(i)) };
}

/** 源码里以字面量出现过的 t() 键（R3 用它把判据收窄到「用户可见」）。 */
function collectReferencedKeys(srcFiles) {
  const set = new Set();
  const KEY = /(?:^|[^A-Za-z0-9_$.])(?:i18n\.)?t\(\s*"([^"]+)"\s*[,)]/g;
  for (const file of srcFiles) {
    for (const line of fs.readFileSync(file, "utf8").split(/\r?\n/)) {
      const s = line.trimStart();
      if (s.startsWith("//") || s.startsWith("*") || s.startsWith("/*")) continue;
      KEY.lastIndex = 0;
      let m;
      while ((m = KEY.exec(line)) !== null) set.add(m[1]);
    }
  }
  return set;
}

const CJK = /[\u4e00-\u9fff]/;

/**
 * R3：值 == 自身键名（末段），且该键被代码字面量引用，且 zh-CN 同键是含汉字的真译文。
 *
 * 为什么不能只看「值==键名」：非 CJK 语言里 `PascalCase`、`copies`、`tokens` 这类标签的
 * **正确写法就与键名同形**。这类合法项逐条进豁免清单（带 locale + key + reason），
 * 清单条目失效即判红 —— 与 R2 同一套防「只增不减」的机制。
 */
function findResidueLeaves(dicts, refLeaves, referenced, allowlist) {
  const hits = [];
  const stale = new Set(allowlist.map((_, i) => i));
  for (const [file, leaves] of dicts) {
    const locale = file.replace(/\.json$/, "");
    if (locale === "zh-CN") continue;
    for (const [p, v] of leaves) {
      if (typeof v !== "string" || v !== p.split(".").pop() || !referenced.has(p)) continue;
      const zv = refLeaves.get(p);
      if (typeof zv !== "string" || zv === v || !CJK.test(zv)) continue;
      const idx = allowlist.findIndex((e) => e.locale === locale && e.key === p);
      if (idx >= 0) {
        stale.delete(idx);
        continue;
      }
      hits.push(`${file}: ${p} = ${v}（zh-CN=「${zv}」）`);
    }
  }
  return { hits, stale: allowlist.filter((_, i) => stale.has(i)) };
}

/** 正控：构造形态必须被对应规则报红/放行，否则判据面被写窄 = 假绿。 */
function selftest() {
  const mk = (obj) => new Map([["x.json", leafPaths(obj, "", new Map())]]);
  const cases = [
    {
      name: "R1 值回吐键路径",
      check: () => findPathEchoLeaves(mk({ a: { b: "a.b" } })).length === 1,
    },
    {
      name: "R1 正常译文不误报",
      check: () => findPathEchoLeaves(mk({ a: { b: "暂无复盘记录" } })).length === 0,
    },
    {
      name: "R2 拼串绕过插值",
      check: () => {
        const leaves = leafPaths({ a: { k: "应用选中项 ({{count}})" } }, "", new Map());
        const src = path.join(ROOT, ".workbuddy", "tmp", "selftest-r2-a.tsx");
        fs.mkdirSync(path.dirname(src), { recursive: true });
        fs.writeFileSync(src, 'export const x = (t) => t("a.k") + " (" + 3 + ")";\n', "utf8");
        try {
          return findUninterpolatedCalls(leaves, [src]).length === 1;
        } finally {
          fs.rmSync(src, { force: true });
        }
      },
    },
    {
      name: "R2 名字写错的 replace 仍报红",
      check: () => {
        const leaves = leafPaths({ a: { k: "{{count}}分钟前" } }, "", new Map());
        const src = path.join(ROOT, ".workbuddy", "tmp", "selftest-r2-b.tsx");
        fs.mkdirSync(path.dirname(src), { recursive: true });
        fs.writeFileSync(src, 'export const x = (t) => t("a.k").replace("{{n}}", "5");\n', "utf8");
        try {
          return findUninterpolatedCalls(leaves, [src]).length === 1;
        } finally {
          fs.rmSync(src, { force: true });
        }
      },
    },
    {
      name: "R2 传了插值参数不误报",
      check: () => {
        const leaves = leafPaths({ a: { k: "应用选中项 ({{count}})" } }, "", new Map());
        const src = path.join(ROOT, ".workbuddy", "tmp", "selftest-r2-c.tsx");
        fs.mkdirSync(path.dirname(src), { recursive: true });
        fs.writeFileSync(src, 'export const x = (t) => t("a.k", { count: 3 });\n', "utf8");
        try {
          return findUninterpolatedCalls(leaves, [src]).length === 0;
        } finally {
          fs.rmSync(src, { force: true });
        }
      },
    },
    {
      name: "R2 豁免命中即放行",
      check: () => {
        const hits = [{ file: "a/b.tsx", key: "a.k" }];
        const r = applyAllowlist(hits, [{ file: "a/b.tsx", key: "a.k", reason: "讲解占位符" }]);
        return r.kept.length === 0 && r.stale.length === 0;
      },
    },
    {
      name: "R2 豁免失效必须报红",
      check: () => {
        const r = applyAllowlist([], [{ file: "a/b.tsx", key: "gone", reason: "已修" }]);
        return r.stale.length === 1;
      },
    },
    {
      name: "R2 键不同名不放行",
      check: () => {
        const r = applyAllowlist([{ file: "a/b.tsx", key: "a.k" }], [{ file: "a/b.tsx", key: "a.other", reason: "x" }]);
        return r.kept.length === 1 && r.stale.length === 1;
      },
    },
    {
      name: "R3 值==键名且被引用 ⇒ 报红",
      check: () => {
        const dicts = new Map([
          ["zh-CN.json", leafPaths({ a: { market: "行情" } }, "", new Map())],
          ["ko.json", leafPaths({ a: { market: "market" } }, "", new Map())],
        ]);
        const r = findResidueLeaves(dicts, dicts.get("zh-CN.json"), new Set(["a.market"]), []);
        return r.hits.length === 1;
      },
    },
    {
      name: "R3 未被代码引用不报（不是用户可见缺陷）",
      check: () => {
        const dicts = new Map([
          ["zh-CN.json", leafPaths({ a: { market: "行情" } }, "", new Map())],
          ["ko.json", leafPaths({ a: { market: "market" } }, "", new Map())],
        ]);
        const r = findResidueLeaves(dicts, dicts.get("zh-CN.json"), new Set(), []);
        return r.hits.length === 0;
      },
    },
    {
      name: "R3 正确写法与键同形时，靠豁免放行；豁免失效即报红",
      check: () => {
        const dicts = new Map([
          ["zh-CN.json", leafPaths({ profile: { options: { PascalCase: "帕斯卡命名" } } }, "", new Map())],
          ["de.json", leafPaths({ profile: { options: { PascalCase: "PascalCase" } } }, "", new Map())],
        ]);
        const ref = dicts.get("zh-CN.json");
        const key = "profile.options.PascalCase";
        const ex = [{ locale: "de", key, reason: "命名法标识符" }];
        const withEx = findResidueLeaves(dicts, ref, new Set([key]), ex);
        const withoutEx = findResidueLeaves(dicts, ref, new Set([key]), []);
        const stale = findResidueLeaves(new Map([["zh-CN.json", ref], ["de.json", new Map()]]), ref, new Set(), ex);
        return withEx.hits.length === 0 && withoutEx.hits.length === 1 && stale.stale.length === 1;
      },
    },
  ];
  let bad = 0;
  for (const c of cases) {
    let ok = false;
    try {
      ok = c.check();
    } catch (e) {
      ok = false;
      c.err = e.message;
    }
    console.log(`${ok ? "PASS" : "FAIL"}  正控 ${c.name}${c.err ? ` (${c.err})` : ""}`);
    if (!ok) bad++;
  }
  console.log(bad === 0 ? "\n正控全部有效" : `\n正控失效 ${bad} 条`);
  process.exit(bad === 0 ? 0 : 1);
}

if (process.argv.includes("--selftest")) selftest();

const maxList = Number.parseInt(argOf("list", "12"), 10);
if (!fs.existsSync(LOCALE_DIR)) {
  process.stderr.write(`check-i18n-placeholder-usage: 找不到 ${LOCALE_DIR}\n`);
  process.exit(1);
}

const dicts = new Map();
for (const f of fs.readdirSync(LOCALE_DIR).filter((x) => x.endsWith(".json")).sort()) {
  let parsed;
  try {
    parsed = JSON.parse(fs.readFileSync(path.join(LOCALE_DIR, f), "utf8"));
  } catch (e) {
    process.stderr.write(`check-i18n-placeholder-usage: ${f} 不是合法 JSON：${e.message}\n`);
    process.exit(1);
  }
  dicts.set(f, leafPaths(parsed, "", new Map()));
}
if (!dicts.has(REF_LOCALE)) {
  process.stderr.write(`check-i18n-placeholder-usage: 基准 ${REF_LOCALE} 不存在\n`);
  process.exit(1);
}

const srcFiles = [...walkSource(SRC_DIR)];
const r1 = findPathEchoLeaves(dicts);
const r2All = findUninterpolatedCalls(dicts.get(REF_LOCALE), srcFiles);

let allowlist = [];
let residueEx = [];
try {
  const raw = JSON.parse(fs.readFileSync(ALLOWLIST_FILE, "utf8"));
  allowlist = raw.exceptions ?? [];
  residueEx = raw["residue-exceptions"] ?? [];
  for (const e of allowlist) {
    if (!e.file || !e.key || !e.reason) {
      process.stderr.write(`check-i18n-placeholder-usage: R2 豁免条目缺 file/key/reason：${JSON.stringify(e)}\n`);
      process.exit(1);
    }
  }
  for (const e of residueEx) {
    if (!e.locale || !e.key || !e.reason) {
      process.stderr.write(`check-i18n-placeholder-usage: R3 豁免条目缺 locale/key/reason：${JSON.stringify(e)}\n`);
      process.exit(1);
    }
  }
} catch (e) {
  if (e.code !== "ENOENT") {
    process.stderr.write(`check-i18n-placeholder-usage: 豁免清单不可解析：${e.message}\n`);
    process.exit(1);
  }
}
const { kept: r2, stale: staleEx } = applyAllowlist(r2All, allowlist);
const exempted = r2All.length - r2.length;
const r3 = findResidueLeaves(dicts, dicts.get(REF_LOCALE), collectReferencedKeys(srcFiles), residueEx);

const total = [...dicts.values()].reduce((n, m) => n + m.size, 0);
console.log(`语言文件 ${dicts.size} 个 / 叶子值 ${total} 条 / 源码文件 ${srcFiles.length} 个\n`);

function report(title, hits) {
  console.log(`${hits.length === 0 ? "OK  " : "FAIL"} ${title}：${hits.length} 处`);
  for (const h of hits.slice(0, maxList)) console.log(`       ${h}`);
  if (hits.length > maxList) console.log(`       … 另有 ${hits.length - maxList} 处`);
}
report("R1 值回吐键路径（界面会显示原始点分路径）", r1);
report(`R2 资源含 {{占位符}} 但调用点漏插值（基准 ${REF_LOCALE}）`, r2.map((h) => h.text));
report("R3 值==键名的未译残留（仅判被代码引用的键）", r3.hits);
if (exempted > 0) console.log(`\n（R2 另有 ${exempted} 处、R3 另有 ${residueEx.length - r3.stale.length} 处按豁免清单放行）`);
report("R4 豁免条目已无命中（应删除）", [
  ...staleEx.map((e) => `R2 ${e.file} ${e.key} —— ${e.reason}`),
  ...r3.stale.map((e) => `R3 ${e.locale} ${e.key} —— ${e.reason}`),
]);

const pass = r1.length === 0 && r2.length === 0 && r3.hits.length === 0 && staleEx.length === 0 && r3.stale.length === 0;
console.log(pass ? "\n结论: 占位符使用干净" : "\n结论: 存在占位符使用缺陷");
process.exit(pass ? 0 : 1);
