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
//   违规 = import 目标 resolve 后落在（owner 比较一律先过 normOwner 归一，理由见该函数注释）
//     · src/stores/feature/<B>Store   且 norm(B) != norm(owner)（B 为剥 Store 后缀的目录名）
//     · src/components/<B>/**         且 norm(B) != norm(owner) 且 norm(B) 不属 norm(NEUTRAL_PREFIXES)
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

/**
 * owner 名归一 —— 2026-11 任务 #168 批 A 的**判据变更**（不是豁免扩容）。
 *
 * 为什么必须归一：owner 有两套命名形 —— 组件目录用 kebab-case，store 文件用 camelCase。
 *   旧判据「全等字符串」把**同一个 owner 的两种写法**判成跨域 ⇒ 实测 22 条假阳性
 *   （23 个 import 实例）：`src/components/stock-analysis/**` 引 `@/stores/feature/stockAnalysisStore`
 *   21 条 + `multi-agent` 引 `multiAgentStore` 1 条 + 同 target 重复 1 条（棘轮键去重后不计）。
 *
 * 安全性判据（**实测零假阴**，动手前独立复量，非推断）：
 *   并集 = 41 个组件一级目录名 ∪ 78 个 stores/feature 名（剥 `Store` 后缀后参与，其中 73 个真带后缀、
 *   5 个不带：executionPhaseMachine / executionToolCallUtils / quant / stockWorkflowChatBridge /
 *   workflowStoreUtils）= 去重 100 个名字 ⇒ 归一后 97 键，同键组（成员 >1）**仅 3 组**，
 *   且每组都「目录形与 store 形同时命中」= 同一模块的两种写法：
 *     llmwiki:        llm-wiki        / llmWiki
 *     multiagent:     multi-agent     / multiAgent
 *     stockanalysis:  stock-analysis  / stockAnalysis
 *   ⇒ 不存在「两个真不同 owner 被归一成同名」的组 ⇒ 归一不会把跨域合并成自引，零假阴。
 *
 * 命名形态计数（决定归一只需抹平 -/_ 与大小写）：
 *   kebab 目录 6（fine-tune / llm-wiki / multi-agent / stock-analysis / stock-workspace / time-travel）
 *   snake 目录 0 · camel 目录 1（dynamicUI）· stores/feature 78 个名含 -/_ 的 **0 个**（100% camelCase）
 *   ⇒ 不加「剥后缀」之类新语义（剥 `Store` 后缀是既有判据，见 adjudicateImport，本批不动）。
 *
 * 作用域（关键，越界即洗白）：
 *   ✔ 只作用于 owner 归属推导与比较 —— owner 支、targetOwner 支、NEUTRAL 那次 includes **三支同批**
 *     （NEUTRAL 若不归一，dynamicUI 会在 NEUTRAL 支与 owner 支口径不一致）。
 *   ✘ 不作用于棘轮键：键仍是 `file + " → " + target`（target = import 字面量原文）。
 *     键里**不含 targetOwner** ⇒ 把归一并入键会让 212 条基线全部失配 ⇒ 那是洗白，不是修判据。
 *   ✘ 不作用于白名单匹配：白名单比的是 `file` 与 import 字面量/前缀，不是目录名（见 main 的过滤段）。
 *
 * ⚠ 本批是判据变更，必须单独成批：**收紧基线（212 → 186）是另一批（批 B）**，
 *   故此处不跑 --update-baseline，212 仍是上限，读数里新增的 22 条「已修复」是**账面而非钉账**。
 */
export function normOwner(name) { return name.toLowerCase().replace(/[-_]/g, ""); }

/** NEUTRAL 判定与 owner 判定同口径（登记面保持原样字面量，归一只发生在比较时）*/
function isNeutralOwner(targetOwner) {
  return NEUTRAL_PREFIXES.some((n) => normOwner(n) === normOwner(targetOwner));
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
    // 三支比较同走 normOwner（owner 支 / targetOwner 支 / NEUTRAL 支）；
    // 但 targetOwner 与 target 在条目里**存原形**，棘轮键不受归一影响（见 normOwner 注释的作用域）。
    const sameOwner = normOwner(j.targetOwner) === normOwner(selfOwner);
    const isViolation = j.kind === "store" ? !sameOwner : !sameOwner && !isNeutralOwner(j.targetOwner);
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

  // ── 归一判据（#168 批 A）正负对照 ──
  // normOwner 自身形态：抹平 -/_ 与大小写，不引入「剥后缀」新语义
  check(normOwner("stock-analysis") === normOwner("stockAnalysis") && normOwner("multi-agent") === normOwner("multiAgent") &&
      normOwner("llm-wiki") === normOwner("llmWiki"), "归一正控：-/_ 与大小写抹平");
  check(normOwner("dynamicUI") === "dynamicui", "归一只是比较口径，不改登记面");
  check(normOwner("stockAnalysisStore") !== normOwner("stockAnalysis"),
    "归一负控：norm 不得顺手剥 Store 后缀（剥后缀是 adjudicateImport 的既有语义）");

  // ① 正控：kebab 目录引同 owner 的 camel store ⇒ 不再假阳性（本批改掉 22 条的那一族）
  const n1 = findViolations({
    file: "src/components/stock-analysis/BacktestPage.tsx",
    content: 'import { useStockAnalysisStore } from "@/stores/feature/stockAnalysisStore";\n',
    owner: "stock-analysis",
  });
  check(n1.length === 0, "归一正控①：stock-analysis 引 stockAnalysisStore 是自引，不得再报");
  const n1b = findViolations({
    file: "src/components/multi-agent/MultiAgentDashboard.tsx",
    content: 'import { A } from "@/stores/feature/multiAgentStore";\n',
    owner: "multi-agent",
  });
  check(n1b.length === 0, "归一正控④a：multi-agent 引 multiAgentStore 同形消除");
  const n1c = findViolations({
    file: "src/components/llm-wiki/X.tsx",
    content: 'import { A } from "@/stores/feature/llmWikiStore";\nimport { B } from "@/components/llmWiki/Y";\n',
    owner: "llm-wiki",
  });
  check(n1c.length === 0, "归一正控④b：第三个碰撞组 llm-wiki/llmWiki 同形消除（store 与 component 两支都归一）");

  // ② 负控：归一≠什么都不报。真跨域必须仍红
  const n2 = findViolations({
    file: "src/components/stock-analysis/BacktestPage.tsx",
    content: 'import { useProviderStore } from "@/stores/feature/providerStore";\n',
    owner: "stock-analysis",
  });
  check(n2.length === 1 && n2[0].kind === "store" && n2[0].targetOwner === "provider",
    "归一负控②：stock-analysis 引 providerStore 仍必须红");
  const n2b = findViolations({
    file: "src/components/multi-agent/X.tsx",
    content: 'import { A } from "@/stores/feature/stockAnalysisStore";\n',
    owner: "multi-agent",
  });
  check(n2b.length === 1, "归一负控④c：multi-agent 引 stockAnalysisStore 仍红（不同 owner 不许被归一合并）");
  const n2c = findViolations({
    file: "src/components/stock-analysis/panels/X.tsx",
    content: 'import { A } from "@/components/stock-workspace/Sidebar";\n',
    owner: "stock-analysis",
  });
  check(n2c.length === 1 && n2c[0].kind === "component",
    "归一负控：两个 kebab 目录（stock-analysis vs stock-workspace）归一后仍不等 ⇒ 仍红");

  // ③ 负控：camel 目录 dynamicUI 的 NEUTRAL 支与 owner 支口径必须一致
  const n3 = findViolations({
    file: "src/components/dynamicUI/Renderer.tsx",
    content: 'import { Tooltip } from "@/components/layout/Tooltip";\nimport { C } from "@/components/common/PageHeader";\n',
    owner: "dynamicUI",
  });
  check(n3.length === 0, "归一负控③a：dynamicUI（camel 目录）引 NEUTRAL 目录仍豁免");
  const n3b = findViolations({
    file: "src/components/dynamicUI/Renderer.tsx",
    content: 'import { A } from "@/components/chat/Bubble";\n',
    owner: "dynamicUI",
  });
  check(n3b.length === 1, "归一负控③b：dynamicUI 引别域（chat）仍红，NEUTRAL 归一不许顺手扩面");
  // NEUTRAL 归一的真正靶心：owner 是 kebab 形时，target 的 camel NEUTRAL 名仍须被豁免
  const n3c = findViolations({
    file: "src/components/stock-analysis/X.tsx",
    content: 'import { R } from "@/components/dynamicUI/DynamicUIRenderer";\n',
    owner: "stock-analysis",
  });
  check(n3c.length === 0, "归一负控③c：kebab owner 引 camel NEUTRAL（dynamicUI）仍豁免（NEUTRAL 支与 owner 支同口径）");
  // ③d NEUTRAL 支归一的「形敏」自证：现存 4 个登记名（layout/common/shared/dynamicUI）没有 kebab 变体，
  //   ⇒ NEUTRAL 归一在今天的真实数据上与旧行为**逐字等价**（键差集「消失 22 条全是 store 支、新增 0」即其证据）。
  //   这条合成用例钉的是「口径必须一致」这个设计约束本身：中性名的任何 -/_ 与大小写变体都算命中，
  //   否则将来有人把中性目录改名成 kebab（dynamic-ui）就会让 NEUTRAL 支与 owner 支分裂成两套口径。
  const n3d = findViolations({
    file: "src/components/stock-analysis/X.tsx",
    content: 'import { R } from "@/components/dynamic-ui/DynamicUIRenderer";\n',
    owner: "stock-analysis",
  });
  check(n3d.length === 0, "归一负控③d：NEUTRAL 按形敏匹配（登记面仍写 dynamicUI，比较时归一），不许出现两套口径");

  // 未合并组自证：gatewayLink（store 剥后缀名）与 gateway / link（两个不同目录）不许归一成同名
  const n4 = findViolations({
    file: "src/components/link/X.tsx",
    content: 'import { A } from "@/stores/feature/gatewayLinkStore";\n',
    owner: "link",
  });
  check(n4.length === 1, "归一负控：link 引 gatewayLinkStore 仍红 ⇒ 归一未把 gatewayLink 与 link 合并");

  // ⑤ 键形态钉住：棘轮键 = file + " → " + import 字面量原文；归一不许渗进 target/targetOwner
  const k1 = findViolations({
    file: "src/components/stock-analysis/BacktestPage.tsx",
    content: 'import { useProviderStore } from "@/stores/feature/providerStore";\n',
    owner: "stock-analysis",
  });
  check(k1.length === 1 && k1[0].target === "@/stores/feature/providerStore" && k1[0].targetOwner === "provider" &&
      k1[0].resolvedTarget === "src/stores/feature/providerStore", "键形态：target/targetOwner 存原形，未归一");
  const k2 = findViolations({
    file: "src/components/multi-agent/X.tsx",
    content: 'import { A } from "@/components/stock-analysis/Panel";\n',
    owner: "multi-agent",
  });
  check(k2.length === 1 && k2[0].target === "@/components/stock-analysis/Panel" && k2[0].targetOwner === "stock-analysis",
    "键形态：kebab 形 target/targetOwner 同样存原文 ⇒ 212 条基线键不会因归一失配");

  if (fail > 0) { return false; }
  console.log(
    "SELFTEST OK：resolve（@/ 与相对路径）/ 自引剥后缀 / 中性目录豁免 / 跨 feature 双向负控 / " +
      "归一正控 3 组同 owner（stockAnalysis·multiAgent·llmWiki）/ 归一负控 5 路（providerStore·跨 kebab·dynamicUI 双口径·gatewayLink 未合并）/ " +
      "norm 不剥后缀 / 棘轮键存原形 全部通过"
  );
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
