#!/usr/bin/env node
// scripts/check-component-neutrality.mjs
// 组件中立性门禁（2026-10-31，审查清单建议 #3 落地，team-lead-15 规格）。
//
// 补的是什么盲区：feature 组件直接 import 其它 feature 的 store / 组件，
//   会把「裁剪某个 feature 就编译失败」的耦合越滚越大（实测 420 条存量跨域引用，
//   其中 210 条是对 layout/common/shared 等共享 UI 基元的正常引用，其余是真跨域）。
//
// 规则（静态 import 图，目标 resolve 后判定；动态 import()/require 不在本门职责）：
//   owner = src/components/<X>/** 的 X（X = shared/layout/common/... 一级目录名）
//   违规 = import 目标 resolve 后落在
//     · src/stores/feature/<B>Store   且 B != owner（B 为剥 Store 后缀的目录名）
//     · src/components/<B>/**         且 B != owner 且 B 不在 NEUTRAL_PREFIXES
//   @/ 别名与 ../ 相对路径统一 resolve 成 src/… posix 路径再判，避免两套口径。
//
// NEUTRAL_PREFIXES（共享 UI 基元 / 纯类型目录，天然跨 feature，**不许被「优化」删掉**）：
//   layout / common / shared / dynamicUI——2026-10-31 实测：Tooltip 69 处、
//   DropdownMenu 25、AntdList 16、MonacoEditor/ErrorBoundary 等，删除判定会把
//   门禁变成 210+ 假阳性灾难。新增中性目录必须在此登记并写明理由。
//
// 白名单 scripts/component-neutrality-allowlist.json：
//   fileExemptions  [{ file, targets: [精确 import 串], reason }] —— 精确到文件+目标
//   targetPrefixExemptions [{ prefix, reason }] —— 全项目生效的目标前缀豁免
//   键刻意**不含行号**（行号随编辑漂移，.i18n-allowlist.json 的腐烂是前车之鉴）。
//
// 棘轮：scripts/component-neutrality-baseline.json 存每条违规 {file, target}，
//   只许减少不许增加——新违规必须当场修（下沉/注入/事件化）或显式登记白名单；
//   修掉存量后跑 --update-baseline 收紧基线（若出现新条目，--update-baseline 拒绝执行）。
//
// 用法：
//   node scripts/check-component-neutrality.mjs                    # 门禁
//   node scripts/check-component-neutrality.mjs --selftest         # 内建正负对照
//   node scripts/check-component-neutrality.mjs --update-baseline  # 收紧棘轮基线
//
// 退出码：0 = 无新增违规（或 selftest 全过）；1 = 有新违规 / selftest 失败。

import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = join(__dirname, "..");
const SRC_COMPONENTS = join(root, "src", "components");

const args = process.argv.slice(2);
const SELFTEST = args.includes("--selftest");
const UPDATE_BASELINE = args.includes("--update-baseline");

const BASELINE_PATH = join(root, "scripts", "component-neutrality-baseline.json");
const ALLOWLIST_PATH = join(root, "scripts", "component-neutrality-allowlist.json");

// ── 共享 UI 基元目录（目标侧中性；改动必须同步头部注释）──
const NEUTRAL_PREFIXES = ["layout", "common", "shared", "dynamicUI"];

// ── 与存量实测同一套 resolve（@/ 别名 + 相对路径 → src/… posix）──
export function resolveImportTarget(specifier, fromFile) {
  if (specifier.startsWith("@/")) { return "src/" + specifier.slice(2); }
  if (specifier.startsWith(".")) {
    const parts = fromFile.split("/").slice(0, -1);
    for (const seg of specifier.split("/")) {
      if (seg === ".") { continue; }
      else if (seg === "..") { parts.pop(); }
      else { parts.push(seg); }
    }
    return parts.join("/");
  }
  return null; // node_modules / 裸包名
}

const FROM_RE = /\bfrom\s*["']([^"']+)["']/g;

/** owner 判定：src/components/<X>/… → X；src/components/<file> → "(root)"；非 components → null */
export function ownerOf(resolvedPath) {
  const m = resolvedPath.match(/^src\/components\/([^/]+)/);
  if (m) { return m[1]; }
  if (/^src\/components\/[^/]+$/.test(resolvedPath)) { return "(root)"; }
  return null;
}

/** 单条 import 判定。返回 null（不涉规则）或 { kind, targetOwner, target }。 */
export function adjudicateImport(resolvedTarget) {
  const sm = resolvedTarget.match(/^src\/stores\/feature\/([^/]+)/);
  if (sm) {
    const b = sm[1].replace(/Store$/, ""); // styleStore → style：自引不算跨域
    return { kind: "store", targetOwner: b, target: resolvedTarget };
  }
  const cm = resolvedTarget.match(/^src\/components\/([^/]+)/);
  if (cm) {
    return { kind: "component", targetOwner: cm[1], target: resolvedTarget };
  }
  return null;
}

/** 纯函数：给定文件内容 → 违规条目列表（selftest 与正式跑共用）。 */
export function findViolations({ file, content, owner }) {
  const violations = [];
  let m;
  FROM_RE.lastIndex = 0;
  while ((m = FROM_RE.exec(content)) !== null) {
    const spec = m[1];
    const resolved = resolveImportTarget(spec, file);
    if (!resolved) { continue; }
    const selfOwner = owner ?? ownerOf(resolved);
    if (!selfOwner) { continue; }
    const j = adjudicateImport(resolved);
    if (!j) { continue; }
    const isViolation =
      j.kind === "store"
        ? j.targetOwner !== selfOwner
        : j.targetOwner !== selfOwner && !NEUTRAL_PREFIXES.includes(j.targetOwner);
    if (isViolation) {
      violations.push({ file, target: spec, resolvedTarget: j.target, kind: j.kind, targetOwner: j.targetOwner });
    }
  }
  return violations;
}

// ── 文件收集 ──
function collectFiles(dir, out) {
  if (!existsSync(dir)) { return out; }
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) { collectFiles(p, out); }
    else if (/\.(ts|tsx)$/.test(name) && !name.endsWith(".d.ts")) { out.push(p); }
  }
  return out;
}

// ── selftest：正负对照（不碰真实 src）──
function runSelftest() {
  let fail = 0;
  const check = (cond, msg) => {
    if (!cond) { console.error(`SELFTEST FAIL：${msg}`); fail++; }
  };

  // resolve：@/ 别名
  check(resolveImportTarget("@/stores/feature/xStore", "src/components/a/b.tsx") === "src/stores/feature/xStore",
    "resolve @/ 别名");
  // resolve：相对路径逐级上跳
  check(resolveImportTarget("../../stores/feature/decompositionStore", "src/components/decomposition/x.tsx") === "src/stores/feature/decompositionStore",
    "resolve ../../ 相对路径");
  check(resolveImportTarget("../gateway/tokenFormat", "src/components/chat/x.tsx") === "src/components/gateway/tokenFormat",
    "resolve ../ 相对路径");
  // resolve：node_modules 不判
  check(resolveImportTarget("react", "src/components/a.tsx") === null, "node_modules 不 resolve");

  // 负控：feature 组件 import 别的 feature 的 store ⇒ 违规
  const v1 = findViolations({
    file: "src/components/chat/AgentExecutionPanel.tsx",
    content: 'import { useExecutionStore } from "@/stores/feature/executionStore";\n',
    owner: "chat",
  });
  check(v1.length === 1 && v1[0].kind === "store" && v1[0].targetOwner === "execution",
    "负控：chat 引 executionStore 必须红");

  // 正控：自引自己的 store（剥 Store 后缀）⇒ 不红
  const v2 = findViolations({
    file: "src/components/style/StylePreviewPanel.tsx",
    content: 'import { useStyleStore } from "@/stores/feature/styleStore";\n',
    owner: "style",
  });
  check(v2.length === 0, "正控：style 引 styleStore 是自引，不得假阳性");

  // 正控：中性 UI 基元目录（layout/common/shared/dynamicUI）⇒ 不红
  const v3 = findViolations({
    file: "src/components/chat/X.tsx",
    content: 'import { Tooltip } from "@/components/layout/Tooltip";\nimport { E } from "@/components/shared/ErrorBoundary";\nimport { R } from "@/components/dynamicUI/DynamicUIRenderer";\n',
    owner: "chat",
  });
  check(v3.length === 0, "正控：NEUTRAL_PREFIXES 目录不得假阳性");

  // 负控：跨 feature 组件引用 ⇒ 违规
  const v4 = findViolations({
    file: "src/components/stock-workspace/ContextSidebar.tsx",
    content: 'import { A } from "@/components/stock-analysis/AnnouncementsPanel";\n',
    owner: "stock-workspace",
  });
  check(v4.length === 1 && v4[0].kind === "component" && v4[0].targetOwner === "stock-analysis",
    "负控：stock-workspace 引 stock-analysis 组件必须红");

  // 正控：非 components 文件不判 owner（null ⇒ skip）
  const v5 = findViolations({
    file: "src/pages/X.tsx",
    content: 'import { S } from "@/stores/feature/agentStore";\n',
    owner: null,
  });
  check(v5.length === 0, "正控：components 目录之外的文件不判");

  if (fail > 0) { return false; }
  console.log("SELFTEST OK：resolve（@/ 与相对路径）/ 自引剥后缀 / 中性目录豁免 / 跨 feature 双向负控 全部通过");
  return true;
}

// ── main ──
if (SELFTEST) { process.exit(runSelftest() ? 0 : 1); }

if (!existsSync(SRC_COMPONENTS)) {
  console.log("⚠ src/components 不存在 —— 跳过 ≠ 通过，本轮未验证组件中立性。");
  process.exit(0);
}

const allowlist = existsSync(ALLOWLIST_PATH)
  ? JSON.parse(readFileSync(ALLOWLIST_PATH, "utf8"))
  : { fileExemptions: [], targetPrefixExemptions: [] };
const fileExemptMap = new Map();
for (const fe of allowlist.fileExemptions ?? []) {
  fileExemptMap.set(fe.file, new Set(fe.targets ?? []));
}
const prefixExemptions = allowlist.targetPrefixExemptions ?? [];

const files = collectFiles(SRC_COMPONENTS, []).map((p) => relative(root, p).split("\\").join("/"));
const allViolations = [];
for (const rel of files) {
  const owner = ownerOf(rel);
  const content = readFileSync(join(root, rel), "utf8");
  for (const v of findViolations({ file: rel, content, owner })) { allViolations.push(v); }
}

// 白名单过滤
const effective = allViolations.filter((v) => {
  const exemptTargets = fileExemptMap.get(v.file);
  if (exemptTargets?.has(v.target)) { return false; }
  if (prefixExemptions.some((pe) => v.target.startsWith(pe.prefix))) { return false; }
  return true;
});

console.log(`\n=== 组件中立性检查 ===`);
console.log(`扫描 ${files.length} 文件 · 原始跨域引用 ${allViolations.length} · 白名单豁免后 ${effective.length}`);

// 棘轮比对
let baseline = { entries: [] };
if (existsSync(BASELINE_PATH)) {
  try { baseline = JSON.parse(readFileSync(BASELINE_PATH, "utf8")); } catch { /* 损坏按空处理 */ }
}
const baseKeys = new Set((baseline.entries ?? []).map((e) => e.file + " → " + e.target));
const curKeys = new Set(effective.map((v) => v.file + " → " + v.target));
const newViolations = effective.filter((v) => !baseKeys.has(v.file + " → " + v.target));
const fixedCount = [...baseKeys].filter((k) => !curKeys.has(k)).length;

if (UPDATE_BASELINE) {
  // 首次建立（基线文件不存在）不受「新违规」保护 —— 没有基线可比，棘轮从当前状态起算；
  // 后续收紧才受保护：存在基线外新违规时拒绝，防止用 update 洗白新增耦合。
  const isFirstEstablish = !existsSync(BASELINE_PATH);
  if (!isFirstEstablish && newViolations.length > 0) {
    console.error(`\nFAIL: --update-baseline 拒绝执行：当前存在 ${newViolations.length} 条基线外新违规，先修或显式登记白名单：`);
    for (const v of newViolations.slice(0, 20)) { console.error(`  + ${v.file} → ${v.target}`); }
    process.exit(1);
  }
  const fsSync = await import("node:fs");
  // 写入前按 file+target 去重：同一文件多行 import 同一 target 会产生重复条目
  const seen = new Set();
  const deduped = effective.filter((v) => {
    const k = v.file + " → " + v.target;
    if (seen.has(k)) { return false; }
    seen.add(k);
    return true;
  });
  fsSync.writeFileSync(
    BASELINE_PATH,
    JSON.stringify({
      note: "棘轮基线：键 = file + target（不含行号，防漂移）。只许减少不许增加；新增违规必须当场修或登记白名单。NEUTRAL_PREFIXES 见 check-component-neutrality.mjs 头部。",
      entries: deduped.map((v) => ({ file: v.file, target: v.target, kind: v.kind, targetOwner: v.targetOwner })),
    }, null, 2) + "\n",
    "utf8",
  );
  console.log(`\n基线已更新：${deduped.length} 条唯一键（本轮已修复 ${fixedCount > 0 ? `较上版减少 ${fixedCount}` : "首次建立"}）。`);
  process.exit(0);
}

if (newViolations.length > 0) {
  console.error(`\nFAIL: ${newViolations.length} 条基线外新违规（修掉、下沉或登记白名单；不许改基线洗白）：`);
  for (const v of newViolations.slice(0, 30)) {
    console.error(`  + ${v.file} → ${v.target}（${v.kind}:${v.targetOwner}）`);
  }
  if (newViolations.length > 30) { console.error(`  … 其余 ${newViolations.length - 30} 条省略`); }
  console.error(`\n若确属误报（如新中性目录），在 scripts/check-component-neutrality.mjs 的 NEUTRAL_PREFIXES 登记并写明理由，不要静默扩豁免。`);
  process.exit(1);
}

console.log(`PASS: 无新增违规（基线 ${baseKeys.size} 条，本轮已修复 ${fixedCount} 条可择机 --update-baseline 收紧）。`);
process.exit(0);
