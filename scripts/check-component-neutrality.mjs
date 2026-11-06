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
//     · src/stores/feature/<B>Store   且 normOwner(B) != normOwner(owner)（B 为剥 Store 后缀的目录名）
//     · src/components/<B>/**         且 normOwner(B) != normOwner(owner) 且 B 未逐字登记在 NEUTRAL_PREFIXES
//   **两支故意不同判据**（2026-11-05 任务 #200）：owner 比较问「是不是同一个模块的两种命名形」⇒ 归一；
//   NEUTRAL 成员资格问「是不是人登记过的中性目录」⇒ 逐字命中登记表。理由见 normOwner 注释的作用域段。
//   @/ 别名与 ../ 相对路径统一 resolve 成 src/… posix 路径再判，避免两套口径。
//
// NEUTRAL_PREFIXES（共享 UI 基元 / 纯类型目录，天然跨 feature，**不许被「优化」删掉**）：
//   layout / common / shared / dynamicUI——2026-10-31 实测：Tooltip 69 处、
//   DropdownMenu 25、AntdList 16、MonacoEditor/ErrorBoundary 等，删除判定会把
//   门禁变成 210+ 假阳性灾难。新增中性目录必须在此**逐字登记并写明理由**（AGENTS.md「组件中立性规范」），
//   不许由命名形态自动推导 —— 否则「把目录改成 kebab 形」就白拿豁免，等于静默扩面。
//
// 空洞通过防护（2026-11-05 任务 #199）：src/components 缺失 / 扫描面塌方 ⇒ **退 1**，不再「跳过 = 通过」。
//   --selftest 全是纯函数断言、不碰真实树，拦不住「扫描器看不见任何文件」这种退化（棘轮会静默变绿）。
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
// 退出码：0 = 无新增违规（或 selftest 全过）；
//         1 = 有新违规 / 扫描面塌方（空洞通过）/ src/components 缺失 / selftest 失败。

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
// 登记表 = **唯一权威**，匹配时逐字命中（精确匹配，不随 normOwner 归一；理由见 normOwner 作用域段）。
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
 *   ✔ 只作用于 **owner 归属推导与比较的两支** —— selfOwner 支与 targetOwner 支（`findViolations` 里的
 *     `normOwner(j.targetOwner) === normOwner(selfOwner)`，两支一起，一字不可拆）。
 *   ✘ **不作用于 NEUTRAL 成员资格**（`isNeutralOwner`）。2026-11-05 任务 #200 把它退回精确匹配，
 *     并**推翻了本文件此前那句「NEUTRAL 不归一会和 owner 支分裂成两套口径」** —— 那句话把两类
 *     语义不同的比较当成了同一件事，是错的。两支故意不同判据：
 *       · owner 比较问「这两个名字是不是**同一个模块**的两种命名形」（目录 kebab vs store camel）
 *         ⇒ 归一是正确判据，#168 批 A 据此消掉 22 条同 owner 假阳性；
 *       · NEUTRAL 问「这个目录名是不是**人登记过**的中性件目录」——AGENTS.md「组件中立性规范」明文
 *         「新增中性目录必须登记进脚本 NEUTRAL_PREFIXES 并写明理由，不许静默扩」。登记是**人的决定**，
 *         命名形态推不出来。若一并归一，把某个 feature 目录改名成 `dynamic-ui` 这种「未登记的 kebab
 *         变体」就**白拿豁免** ⇒ 扩面动作从「改一行登记表 + 写理由 + 被评审」退化成「改目录名」，
 *         白名单与棘轮同时形同虚设。
 *     真实数据上收窄的影响为 **0**（已独立复量：现存 4 个登记名没有任何 kebab/snake 变体出现在
 *     import 图里，判定档读数逐字不变）⇒ selftest ③d 因此可以从正控翻成负控而不改账面。
 *   ✘ 不作用于棘轮键：键仍是 `file + " → " + target`（target = import 字面量原文）。
 *     键里**不含 targetOwner** ⇒ 把归一并入键会让 212 条基线全部失配 ⇒ 那是洗白，不是修判据。
 *   ✘ 不作用于白名单匹配：白名单比的是 `file` 与 import 字面量/前缀，不是目录名（见 main 的过滤段）。
 *
 * ⚠ 本批是判据变更，必须单独成批：**收紧基线（212 → 186）是另一批（批 B）**，
 *   故此处不跑 --update-baseline，212 仍是上限，读数里新增的 22 条「已修复」是**账面而非钉账**。
 */
export function normOwner(name) { return name.toLowerCase().replace(/[-_]/g, ""); }

/**
 * NEUTRAL 成员资格 = **逐字命中登记表**，不随 normOwner 归一（2026-11-05 任务 #200 收窄，leader 裁定）。
 * 为什么与 owner 两支故意不同判据（而不是「口径分裂」）：见 normOwner 注释的作用域段 ——
 * owner 支问「是不是同一个模块的两种写法」，本支问「是不是登记过的中性目录」，后者是人的决定。
 * 钉住它的两条 selftest 断言：③c（登记名原形 dynamicUI 仍豁免，正控）、
 * ③d（未登记的 kebab 变体 dynamic-ui 必须报违规，负控）。
 */
function isNeutralOwner(targetOwner) {
  return NEUTRAL_PREFIXES.includes(targetOwner);
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
    // owner 两支（selfOwner / targetOwner）同走 normOwner 归一；NEUTRAL 支逐字命中登记表（#200）。
    // 两支故意不同判据的理由见 normOwner 注释的作用域段，不要「顺手统一」成三支一起归一或一起全等。
    // targetOwner 与 target 在条目里**存原形**，棘轮键不受归一影响（见 normOwner 注释的作用域）。
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

/**
 * 空洞通过（hollow pass）防护 —— 2026-11-05 任务 #199。
 *
 * 拦的是什么：这道门是**棘轮**，判据是「本轮违规集合 ⊆ 基线集合」。扫描器一旦看不见文件
 *   （`src/components` 被改名/搬迁、根路径写错、后缀口径退化、构建产物目录被清空），
 *   违规集合就是空集 ⇒ 空集是任何集合的子集 ⇒ **PASS 退 0**。此后新写的跨域引用永远不会被拦，
 *   门从棘轮静默退化成「永远绿」。CI 里 `--selftest` 先跑的那一步**救不了它**：selftest 全是
 *   纯函数断言、不碰真实树（见 runSelftest 头注释「不碰真实 src」），扫描器瞎掉时它照样全绿。
 *
 * 双条件锚（**都不是魔法数**，两条各自兜住对方退化的那一半）：
 *   a) 绝对下界 `scannedFilesCount >= MIN_SCANNED_FILES`：
 *      兜住「基线缺失/损坏/为空 ⇒ 条件 b 退化成 `>= 0`」这一支（历史上基线文件曾不存在，见 --update-baseline
 *      的 isFirstEstablish 分支，那不是假想情形）。
 *      取值理由：2026-11-05 实测扫描面 = **927** 个 .ts/.tsx（本文件尾部报告即读数量）。
 *      取 **1/3 向下取整到百位 = 300**：允许一次「把组件拆到别处 / 大规模合并目录」的重构（账面腰斩两次
 *      仍不误红），但拦住任何数量级的塌方（0、个位数、只剩一个子目录）。锚是**比例**不是瞬时读数，
 *      所以不随每次提交漂移；漂移的是报告里那个 927。
 *   b) 相对锚 `scannedFilesCount >= 基线里出现的不同 file 数`（仅基线非空时生效）：
 *      这条**随棘轮自更新**，是人写的数字里唯一机械可解释的那个：212 条基线键覆盖 **157** 个不同
 *      `src/components/**` 文件，而每一条键的 file 都必然来自某一轮真实扫描 ⇒「扫描面」的下界
 *      天然就是「基线面」。扫描面掉到基线面以下 ⇒ 有一批**仍在被账面保护的引用所在的文件根本没被读到**
 *      ⇒ 判定不作数。收紧基线时该下界同步下降，不需要有人记得改常量。
 *      为什么用**计数**而不是「基线 file ⊆ 扫描 file」的集合包含：实测 157 个基线 file 里已有 **2 个
 *      在磁盘上不存在**（存量修复时文件被删/搬走，棘轮键残留属正常 decay，靠 --update-baseline 收干），
 *      集合包含会当场误红。计数只问「读到的面积够不够」，不问「每一个历史 file 是否还在」，
 *      正好是要的语义强度。
 *
 * 纯函数，不读盘 ⇒ selftest 可以用注入数字做正负控（真实树上「927 ≥ 300」是永真断言，钉不住退化）。
 */
const MIN_SCANNED_FILES = 300;

export function scanFloorFailures(scannedFilesCount, baselineFilesCount) {
  const problems = [];
  if (scannedFilesCount < MIN_SCANNED_FILES) {
    problems.push(
      `条件 a（绝对下界）：本轮只扫描到 ${scannedFilesCount} 个 .ts/.tsx，< 下界 ${MIN_SCANNED_FILES}` +
        `（2026-11-05 实测扫描面 927 的 1/3）—— 扫描面塌方时「0 条新违规」不代表「没有违规」。`,
    );
  }
  if (baselineFilesCount > 0 && scannedFilesCount < baselineFilesCount) {
    problems.push(
      `条件 b（相对基线面）：本轮扫描到 ${scannedFilesCount} 个不同 file < 棘轮基线里出现的 ` +
        `${baselineFilesCount} 个不同 file —— 有仍在基线账面里的文件没被读到，判定不作数。`,
    );
  }
  return problems;
}

// ── selftest：正负对照（不碰真实 src）──
// ⚠ 本函数**只测判据逻辑**，从不读真实文件树 ⇒ 它拦不住「扫描器看不见任何文件」这类退化，
//   那件事由 main 里的空洞通过防护（scanFloorFailures）负责；本函数用**注入数字**验它自己的判据（⑥）。
function runSelftest() {
  let fail = 0;
  let total = 0;
  const check = (cond, msg) => {
    total++;
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

  // ③ NEUTRAL 支（精确匹配，#200）与 owner 支（归一，#168）两套判据**各自**钉死
  const n3 = findViolations({
    file: "src/components/dynamicUI/Renderer.tsx",
    content: 'import { Tooltip } from "@/components/layout/Tooltip";\nimport { C } from "@/components/common/PageHeader";\n',
    owner: "dynamicUI",
  });
  check(n3.length === 0, "NEUTRAL 正控③a：dynamicUI（camel 目录）引登记名 layout/common ⇒ 豁免");
  const n3b = findViolations({
    file: "src/components/dynamicUI/Renderer.tsx",
    content: 'import { A } from "@/components/chat/Bubble";\n',
    owner: "dynamicUI",
  });
  check(n3b.length === 1, "NEUTRAL 负控③b：dynamicUI 引未登记域（chat）仍红，中性豁免面不许靠命名形态自动扩");
  // ③c 登记面**原形**必须仍被豁免（收窄 NEUTRAL 支后仍如此 —— 这条是「精确匹配没把 4 个登记名判漏」的正控）
  const n3c = findViolations({
    file: "src/components/stock-analysis/X.tsx",
    content: 'import { R } from "@/components/dynamicUI/DynamicUIRenderer";\n',
    owner: "stock-analysis",
  });
  check(n3c.length === 0, "NEUTRAL 正控③c：kebab owner 引登记名原形 dynamicUI 仍豁免（登记表逐字命中即可）");
  // ③d 负控（**#200 判据反转**：#168 批 A 曾把这条钉成正控「dynamic-ui 也算中性」）：
  //   `dynamic-ui` **不在** NEUTRAL_PREFIXES 登记表里 ⇒ 必须报违规。中性目录的扩面是人的登记决定
  //   （AGENTS.md「组件中立性规范」：新增必须登记 + 写明理由，不许静默扩），不许由命名形态推导出来。
  //   真实数据影响为 0：现存 4 个登记名没有任何 kebab/snake 变体出现在 import 图里，
  //   故本条从正控翻成负控时判定档读数逐字不变（2026-11-05 复量：202 / 199 / PASS 基线 212）。
  const n3d = findViolations({
    file: "src/components/stock-analysis/X.tsx",
    content: 'import { R } from "@/components/dynamic-ui/DynamicUIRenderer";\n',
    owner: "stock-analysis",
  });
  check(n3d.length === 1 && n3d[0].targetOwner === "dynamic-ui",
    "NEUTRAL 负控③d：未登记的 kebab 变体 dynamic-ui **必须**报违规 ⇒ 中性扩面只能改 NEUTRAL_PREFIXES 登记表，改名白拿豁免即红");
  // ③e 正控（#200 补，与 ③d 互为犄角）：owner 两支**仍归一**不许被「一起退回全等」。
  //   同一文件里同时命中 store 支（stockAnalysisStore）与 component 支（camel 目录 stockAnalysis）的
  //   同 owner 两种写法（n1 只钉 store 支、n1c 只钉 llm-wiki 组），下一次有人把三支一起改成全等 ⇒ 本条立刻红
  //   （③d 只守 NEUTRAL 那一支，守不了这两支）。
  const n3e = findViolations({
    file: "src/components/stock-analysis/BacktestPage.tsx",
    content: 'import { A } from "@/stores/feature/stockAnalysisStore";\nimport { B } from "@/components/stockAnalysis/Panel";\n',
    owner: "stock-analysis",
  });
  check(n3e.length === 0,
    "owner 支仍归一③e（正控）：kebab 目录 stock-analysis 引同 owner 的 camel store 支 + camel 组件支两支都豁免 ⇒ 三支一起退回全等即红");

  // ⑥ 空洞通过防护（#199）的判据本身 —— 注入数字做正负控，不碰真实树
  //   （真实树上「927 ≥ 300」是永真断言，扫描器瞎掉时也不会变，钉不住退化）。
  check(scanFloorFailures(927, 157).length === 0,
    "空洞防护正控：真实树实测形状（扫描 927 file / 基线 157 个不同 file）必须放行");
  check(scanFloorFailures(MIN_SCANNED_FILES, 0).length === 0,
    "空洞防护正控：恰好落在下界 + 基线为空（b 退化）⇒ 放行，证明下界不含越界强度");
  check(scanFloorFailures(0, 0).length === 1,
    "空洞防护负控 a：扫描 0 file 且基线为空 ⇒ 仍红（条件 a 兜住 b 退化成 >= 0 的那一支）");
  check(scanFloorFailures(MIN_SCANNED_FILES - 1, 0).length === 1,
    "空洞防护负控 a′：扫描数恰为下界-1 ⇒ 红（边界钉死在「< 下界」，不许松成「只剩一个子目录也算正常」）");
  const floorB = scanFloorFailures(1000, 1001);
  check(floorB.length === 1 && floorB[0].includes("条件 b"),
    "空洞防护负控 b：条件 a 已过（1000 ≥ 300）但基线面 1001 > 扫描面 1000 ⇒ 红，且报的是条件 b ⇒ 两条锚各自独立生效");

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
    `SELFTEST OK（共 ${total} 条断言 · 失败 ${fail}）：` +
      "resolve（@/ 与相对路径）/ 自引剥后缀 / 跨 feature 双向负控 / " +
      "owner 两支归一正控 3 组同 owner（stockAnalysis·multiAgent·llmWiki）+ ③e 双支合钉 / " +
      "owner 归一负控（providerStore·跨 kebab·multiAgent×stockAnalysis·gatewayLink 未合并）/ " +
      "NEUTRAL 精确匹配（③c 登记名豁免·③d 未登记变体必须红）/ norm 不剥后缀 / " +
      "棘轮键存原形 / 空洞通过防护双锚（a 绝对下界·b 相对基线面）全部通过"
  );
  return true;
}

// ── main ──
if (SELFTEST) { process.exit(runSelftest() ? 0 : 1); }

if (!existsSync(SRC_COMPONENTS)) {
  console.error(`FAIL: src/components 不存在（应含本仓全部组件目录）—— 本轮对组件中立性**零验证**，` +
    `「没读到」不等于「没有违规」，门禁按红处理。`);
  console.error(`  两种可能，都必须有人当场判：① 目录被改名/搬迁 ⇒ 同步本文件的 SRC_COMPONENTS 并复核扫描数；` +
    `② 构建/裁剪产物 ⇒ 门禁不该在无源码的树里跑（此时 CI 该 skip 整个 step，而不是让它报绿）。`);
  process.exit(1);
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

// 读取棘轮基线（下面的空洞通过防护的条件 b 要用它的 file 面积，故先于比对读）
let baseline = { entries: [] };
if (existsSync(BASELINE_PATH)) {
  try { baseline = JSON.parse(readFileSync(BASELINE_PATH, "utf8")); } catch { /* 损坏按空处理 —— 由下面的条件 a 兜住 */ }
}

// ── 空洞通过防护（#199）：必须早于棘轮比对 / PASS 输出 / --update-baseline 写入 ──
// 扫描面塌方时 effective 会是空集，空集 ⊆ 任何基线 ⇒ 后面所有分支都会「正确地」报绿；
// 更糟的是 --update-baseline 会照抄这个空集，把 212 条棘轮一次洗成 0 条。所以在这里就叫停。
const scannedFileCount = new Set(files).size;
const baselineFileCount = new Set((baseline.entries ?? []).map((e) => e.file)).size;
const floorProblems = scanFloorFailures(scannedFileCount, baselineFileCount);
if (floorProblems.length > 0) {
  console.error(`\nFAIL: 扫描面塌方 —— 「0 条新违规」不等于「没有违规」，本轮判定不作数：`);
  for (const p of floorProblems) { console.error(`  · ${p}`); }
  console.error(`  先恢复被扫描的源码面 / 修 collectFiles 口径；**禁止**用 --update-baseline 把空集写进基线洗账面。`);
  process.exit(1);
}
// 棘轮比对
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
