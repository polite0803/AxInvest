#!/usr/bin/env node
// scripts/check-capability-register.mjs
// 能力接缝登记表对账门禁（第五道门，2026-11-01，PLAN §0.12.4 裁定「建」/ 任务 #38）。
//
// 建门理由：`docs/plans/PLAN-shell-plus-plugins.md` §0.7「提供方接缝（L2）登记表」是 P5/P6
//   全部判据的分母，而它此前**只靠人工复扫** —— §0.12.4 那轮自查实测抓出
//   **15 处数字漂移 + 2 套并行口径**（1534/1541、工作流 124/83、股票 304/306、前端 60 757/84 873），
//   证明「数字漂了不会有人报警」不是假设。本门复跑两份 audit 的口径与 §0.7.1 声明的扫描面，
//   对机器镜像 `scripts/capability-register.json`（PLAN §0.7 的逐字镜像）做**逐行对账**；
//   读数变化必须显式改表（同批改 PLAN），否则红。
//
// 判据分档：
//   · hard:true 锚点/登记项不一致 ⇒ 红（退出码 1）；
//   · hard:false（register_commands.rs 行数、K1 外部实现者 232、K2 trait 面 51）⇒ 印出但不决定退出码；
//   · entries[].check="manual" ⇒ 不判红，**也不算通过** —— 计入「未机器复检」摘要并逐条印出。
//   ⚠ 例外（2026-11-02 收紧）：**锚点自相矛盾一律硬失败，不看 anchor.hard** —— 那是「表自身散架」，
//     与分组计数不符同档；否则 hard:false 的行可以自说自话（假绿洞①，见 checkAnchorConsistency）。
//   镜像表本身是唯一权威：**无增量基线、无 --update-* 洗白路径**（存在洗白路径 = 漂移可以再被
//   「合法化」，本门失去意义）；--dump-readings 只读调试，不落任何盘。
//
// 数据文件格式（capability-register.json）：note / caliber / sources / countRule / anchors / pipeline / entries。
//   entries[].planAnchorText / planAnchorTextRange = 该登记项 planLine（含区间端点）**登记当时的整行逐字**，
//   只作 --repin-plan 的定位键，**不参与判定档**（判据仍然只认行号 + token ⇒ 加它不会放松任何红）。
//   anchors[].check ∈ registryCommandCount registryModuleCount registryUnassigned registryFileLineCount
//     registryBoundaryCount registryDomainCount frontendVolume frontendDirVolume l1ForeignBlockers
//     k1RsFileCount k1PubTraitCount remoteSeamRegistryCount registerGroupSum manual
//   entries[].check ∈ implementorCount seamExists evidenceOnly manual。
//   ⚠ 为什么登记项的**身份键是 seamId / trait 名而不含行号**，行号只出现在 evidence 锚点里：
//   `.i18n-allowlist.json` 拿行号当键、随编辑整体腐烂是前车之鉴（见 check-i18n-exemption-budget.mjs 头注）。
//   证据锚点的行号**漂移本身就是要抓的缺陷**（登记与盘上脱节），所以它必须判红，而不是被当键使用。
//
// 强制复用：`scripts/lib/command-domains.mjs` 的 REG_PATH / parseRegistry / domainOf / unassigned
//   （AGENTS.md 禁区 12 —— 注册表解析规则全仓唯一，禁止另写第二套正则）。
//   边界口径的 DOMAINS（cmdRe/feRe/useRe/ownedRe）**逐字抄自** `scripts/audit-l1-blockers.mjs:36-51`
//   —— 该审计脚本不导出符号且禁改，抄录是本门与其唯一合法的同步点；改那边正则必须同改这里。
//
// 扫描面（按 sources.scanSurface 显式选边；**一次目录遍历 + 内容按需缓存**，否则共享目录上不可用）：
//   · K1/实现者计数：8 个 workspace 根的全部 .rs，忽略路径段 target / node_modules / gen / tests；
//   · 命令/域/前端/L1 读数：audit-l1-blockers.mjs 的忽略集（target node_modules output .git .tmp
//     dist gen build coverage .workbuddy examples）。两份忽略集**不同**（tests 只属 K1 侧，
//     examples/build 等只属 audit 侧），故目录树只走一遍（仅剪体积目录），按路径段分类选边。
//
// 旧读数（planOld：1534、124）的合法出现位置 —— **显式判定规则（非隐性约定）**：
//   定位 §0.12.4 标题行（/^#{1,4}…0\.12\.4/）到下一个同级或更高一级标题之间的**区段**，
//   且该区段文本含「漂移」字样；旧读数的每次数字命中必须落在该区段内（数字按「非行号引用」
//   边界匹配：前后不接数字，左侧不接 `: . / #`，以免 `PLAN:124` 这类行号引用误判）。
//   区段外的任何出现 ⇒ 红。这就是「漂移列举段」的机械定义，勿在别处再发明第二套。
//
// planMustAppear 的命中口径（2026-11-02 收紧，见 planNumberHits）：**数字边界匹配**，不是裸子串 ——
//   千位空格写法算命中（`84 873`），日期/时刻串整串涂白，前后紧邻数字/小数点不算，左侧紧邻
//   `: / #` 的「行号引用 / 斜杠配对旧数」不算。现盘 15 个 planMustAppear 锚点在新口径下仍全命中。
//
// 锚点内部自洽（2026-11-02 新增硬判据，见 checkAnchorConsistency）：同一锚点的 `value` /
//   `planNumber` / `planPinned[].mustContain`（纯数字 token）必须指同一个数，否则**一律硬失败**
//   （不看 anchor.hard）。补的是旧门的假绿洞：只改 `value` 迁就盘上、PLAN 原文与 planNumber/钉句
//   留旧数，旧门三档判据彼此不比，能一路判绿。
//   ⚠ ②档「跨锚点宽容」的来源**收窄为且仅为** `hard:true` 且被机器复检（check ≠ "manual"）的锚点
//   登记数（value/value2）。理由见 checkAnchorConsistency 头注：软锚点改值永不判红，收它们的数字
//   等于给「把旧数藏进软锚点」留了洗白钉句的后门（假绿洞②，2026-11-11 审查员裁定阻断 1）。
//
// 自检覆盖度（2026-11-02 新增，见 ANCHOR_CHECK_NEGATIVE_CASES）：镜像表用到的**每一类** anchor
//   check 都必须有「登记≠实测 ⇒ 必被抓出」的负控 + 「登记=实测 ⇒ 不假阳性」的正控（全走合成读数，
//   零盘扫描）。--selftest 每次印「已覆盖 check 种类 N / 表内实际使用 M」，出现空洞即自检红。
//
// 用法：
//   node scripts/check-capability-register.mjs                  # 门禁：复算全部实测读数并与镜像表逐行对账
//   node scripts/check-capability-register.mjs --selftest       # 内建正负对照（纯函数、零盘扫描、先于任何 IO 短路）
//   node scripts/check-capability-register.mjs --dump-readings  # 只读调试：打印实测读数（不写盘）
//   node scripts/check-capability-register.mjs --repin-plan     # 维护模式：按定位键把 PLAN 行号引用重锚到当前文本（写盘前先备份）
//
// ── --repin-plan 的由来与判据（任务 #48，2026-11-02）─────────────────────
// 本门用**绝对行号**钉 PLAN（`anchors[].planPinned[].line` + `mustContain`、`entries[].planLine` /
// `planLineRange`）。已实测的代价：主代理把 §0.7.6 从 5 行改成 4 行 ⇒ 全文少 1 行 ⇒ 8 处钉句**同时假红**
// （352/515/500/939/508/507/502/320），补回同数行才转绿 ⇒ 门禁事实上给一个**活文档**施加了一条
// 没写在任何地方的约束「编辑必须行数中性」。假红的代价是训练人去忽略红，所以把它变成一条**可执行的工序**：
// 改 PLAN ⇒ 普通运行红 ⇒ 跑 `--repin-plan` ⇒ 表回写 ⇒ 再跑普通运行。
//
// 判据（三档，全部 fail-closed）：
//   ① **定位键唯一才回写**：钉句按 `mustContain` 全部 token 同行命中；指针按 `planAnchorText`
//      （登记时的整行逐字，仅折叠行内连续空白）全等。命中 1 行 ⇒ 回写该行号；命中 0 或多行 ⇒
//      **整次拒绝写盘**并逐条报告（不许猜行、不许插值、不许「取最近的一个」）。
//   ② **红线：绝不用模糊匹配代替行号锚点**。判定跑（`checkPlan`）仍按「第 N 行是否含该句」逐行核 ——
//      句子漂到别的行、或整句被改写，仍是红。重锚只改**表里的数字**，不改判据形态。
//      这条由负控㉑g 机械锁定：把钉句指到「同一句话出现的另一行」必须照样判红。
//   ③ **缺定位键即拒绝**：钉句无 token、登记项无 `planAnchorText` ⇒ 判 blocker（逼着「新增行号引用
//      必须同批带定位键」，否则重锚能力会随表增长而静默腐烂）。
//
// 写盘工序：先把整份表**逐字节备份**到 `.tmp/capability-register.<UTC 时间戳>.bak.json`，再按定点
// 文本补丁回写（不整体重新序列化 ⇒ 未涉及的字节逐字不动，与并行编辑者的表不互相打散格式），
// 回写后**从盘上重读**并复跑 `checkSelfConsistency` + `checkAnchorConsistency` + `checkPlan`
// 作为复检；复检不过 ⇒ 用备份**回滚**并以非零退出。全程逐条打印「哪个引用 · 旧行号 → 新行号」，
// 未变动的引用给计数、覆盖不到的引用（`caliber.basisLines` / `pipeline.lines` 是纯文档指针，
// 无定位键 ⇒ 本模式不重锚）单独点名，不装作覆盖到了。
//
// ⚠ 本模式**不接进 CI**（`ci-check.mjs` / `ci.yml` 都只有 `--selftest` + 判定两步）：
//   门禁的意义是「漂了就红、由人确认后再改表」，把自动回写挂进 CI 等于让漂移自我合法化。
//
// 退出码：0 = hard:true 判据全部一致（或 selftest 全过 / 重锚写盘并复检通过 / 无需重锚）；
//   1 = 任一 hard:true 不一致 / 镜像表缺失 / selftest 失败 / 重锚被歧义或零命中阻断（未写盘）。
//   hard:false 与 manual 一律不影响退出码（但必须印出来 —— 「未机器复检」不是「通过」）。
//
// main guard（2026-11-11 阻断 2）：本模块的判据**全是 export 的纯函数**，主流程收在 `main()` 里，
//   只在「本文件被直接执行」时运行（`import.meta.url === pathToFileURL(process.argv[1]).href`）。
//   此前顶层即扫描 + `process.exit` ⇒ 任何 `import` 本模块复算纯函数的行为（审查 / 自检 / 纸面实证脚本）
//   都会被劫持跑一次全仓遍历并强制结束进程（实测被劫持跑过 33.3s 全门）。现被 import 时**零副作用、
//   绝不调用 process.exit**；三条命令行（判定 / --selftest / --dump-readings）行为与退出码逐字不变。
//
// 遍历抖动（2026-11-11，同阻断 2 一并落实）：`k1RsFileCount` 是 hard:true，而并发会话正在增删 `.rs`
//   ⇒ 同一趟遍历的头尾可能数到不同文件清单，门禁会因**与登记无关的盘上并发**随机判红/随机判绿。
//   规则：判定跑完后按**同一枚举口径**重新统计一次扫描面文件清单数，与本趟开始时不一致 ⇒ 印
//   「⚠ 遍历期间盘上文件数变化（X→Y），本轮不判 —— 跳过 ≠ 通过」并**重试一次**；重试后仍不收敛
//   ⇒ 按本仓既有门的 skip 语义处理（印警告 + 计入 skipped 摘要），**该轮不因此判红**，exit 码由
//   其余硬失败决定。理由：抖动不是「登记与盘上不符」，判它红是噪声；但也不许静默吞掉 ——
//   「本轮不判」必须显式可见（跳过 ≠ 通过）。代价上界：稳定路径只多一次**纯目录枚举**（不读文件内容），
//   抖动路径最多再跑一轮复算；`--selftest` 零盘扫描，不受影响。

import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { REG_PATH, parseRegistry, domainOf, unassigned } from "./lib/command-domains.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const SELFTEST = args.includes("--selftest");
const DUMP = args.includes("--dump-readings");
const REPIN = args.includes("--repin-plan");

const REGISTER_PATH = join(ROOT, "scripts", "capability-register.json");

// ── 口径常量（与数据文件/审计脚本的同步点都在注释里钉死）────────────────
const K1_ROOTS = ["axagent-ai", "axagent-core", "axagent-infra", "axagent-kit", "axagent-plugins", "axagent-quant", "axagent-runtime", "src-tauri"];
const K1_IGNORE = new Set(["target", "node_modules", "gen", "tests"]); // PLAN §0.7.1 扫描面（= .tmp/capability-census.cjs）
const AUDIT_SKIP = new Set(["target", "node_modules", "output", ".git", ".tmp", "dist", "gen", "build", "coverage", ".workbuddy", "examples"]); // audit-l1-blockers.mjs:17
const WALK_PRUNE = new Set(["target", "node_modules", "gen", ".git"]); // 一次遍历：只剪「体积/必然无料」目录，其余按路径段分类
const HARNESS_PREFIX = "axagent-core/crates/harness/";

// 锚点 check 种类的**声明清单**（与文件头 data-format 行同源，逐字一致）。
// --selftest 用它兜底并反向校验镜像表：表里出现本清单之外的新 check 种类 ⇒ 自检红，
// 逼着「新增一类判据」必须同批补一对负控+正控（第 2 条修复：覆盖度不许再靠惯性维持）。
const ANCHOR_CHECK_KINDS_DECLARED = [
  "registryCommandCount", "registryModuleCount", "registryUnassigned", "registryFileLineCount",
  "registryBoundaryCount", "registryDomainCount", "frontendVolume", "frontendDirVolume",
  "l1ForeignBlockers", "k1RsFileCount", "k1PubTraitCount", "remoteSeamRegistryCount",
  "registerGroupSum", "manual",
];

// DOMAINS 逐字抄自 scripts/audit-l1-blockers.mjs:36-51（152/28 与 293/84 873 的基线出处）
const DOMAINS = {
  STOCK: {
    crates: ["axagent-astock-data", "axagent-analysis-engine", "axagent-quant", "axagent-market-sim"],
    useRe: /axagent_(astock_data|analysis_engine|quant|market_sim)::/,
    ownedRe: /^src-tauri\/src\/(commands|init|bin)\/(stock|market_|backtest|quant|pool_scan|universe_scan|time_travel|paper|trade|conditional_order|price_alert|analyst|recommendation_cron|mover_recall|limit_up|signal|reco|wf_des)/,
    feRe: /stock|quote|kline|technical|portfolio|trading|advis|invest|quant|market/i,
    cmdRe: /^(stock|quote|kline|quant|backtest|portfolio|market_|pool|universe|reco|signal|north|margin|dragon|limit_up|time_travel|advis|paper|trade|conditional_order|price_alert|analyst|fundamental|techn|wf_des)/,
  },
  OPC: {
    crates: ["axagent-company-runtime"],
    useRe: /axagent_company_runtime::/,
    ownedRe: /^src-tauri\/src\/(commands|init)\/(opc|capability_pack|agency|domain_pack|horizon|dojo|demand_discovery)/,
    feRe: /opc|capability-?pack|agency|horizon|domain-?pack|dojo/i,
    cmdRe: /^(opc|capability_pack|agency|domain|horizon|dojo|demand_)/,
  },
};

// ── 纯函数判据（--selftest 与正式跑共用同一实现，禁止两套）────────────────

/** 表自身一致性：分组计数=caliber.groupCounts、entries.length=totalRows、Σ 能力面=capacityRegister。 */
export function checkSelfConsistency(register) {
  const problems = [];
  const { caliber } = register;
  const groups = { A: 0, B: 0, C: 0, H: 0, K: 0 };
  for (const e of register.entries) {
    if (!(e.group in groups)) { problems.push(`未知分组「${e.group}」（seamId=${e.seamId}）`); continue; }
    groups[e.group]++;
  }
  for (const g of Object.keys(groups)) {
    if (groups[g] !== caliber.groupCounts[g]) {
      problems.push(`分组 ${g}：登记 ${caliber.groupCounts[g]} / 实测表内 ${groups[g]} / 差 ${groups[g] - caliber.groupCounts[g]}`);
    }
  }
  if (register.entries.length !== caliber.totalRows) {
    problems.push(`表总行数：登记 ${caliber.totalRows} / 实测 ${register.entries.length} / 差 ${register.entries.length - caliber.totalRows}`);
  }
  const capSum = caliber.groupCounts.A + caliber.groupCounts.B + caliber.groupCounts.C;
  if (capSum !== caliber.capacityRegister) {
    problems.push(`能力面 Σ(A+B+C)：caliber.capacityRegister ${caliber.capacityRegister} / 分组和 ${capSum} / 差 ${capSum - caliber.capacityRegister}`);
  }
  const anchor = register.anchors.find((a) => a.check === "registerGroupSum");
  if (anchor && anchor.value !== capSum) {
    problems.push(`锚点 ${anchor.id}：登记 ${anchor.value} / 分组和 ${capSum} / 差 ${capSum - anchor.value}`);
  }
  return problems;
}

/** 锚点内部自洽（第 1 条修复 2026-11-02，堵假绿洞）：同一锚点的 `value` / `planNumber` /
 *  `planPinned[].mustContain` 必须指**同一个数**。旧门只比「登记值 vs 盘上实测」，从不比锚点自己 ⇒
 *  可以只把 `value` 改成盘上实测值迁就，而 PLAN 原文与 `planNumber`/`mustContain` 还留着旧数，
 *  一路合法判绿（读数漂移被「半改」洗白）。机械判据：
 *   ① 有 `planNumber` 且它是纯数字串（可含千位空格）者：剥掉空格后必须逐字等于 `String(value)`；
 *   ② `planPinned[].mustContain` 里凡是**纯数字串**（含 `84 873` 这类千位空格写法）的 token，
 *      规范化后必须等于**本锚点**的 `value`/`value2`，或等于**跨锚点宽容集合**里的数 —— 后一档宽容
 *      是必需的：现表 PLAN:352 的钉句同时含兄弟锚点读数（「1541 条 / 164 模块」）、PLAN:247 同时钉
 *      2005 与 316、PLAN:508 钉 293（= 本锚点 value2），只按「必须等于 String(value)」死判会把既存
 *      正确写法一并判红（本轮约束：不许新增红）。两侧都对不上 ⇒ 要么 pin 留旧数、要么 value 被单方面
 *      改走 ⇒ 红。
 *      ⚠ **宽容集合的口径（2026-11-11 阻断 1 收紧）**：跨锚点宽容只认 `hard:true` 且机器复检的锚点数；
 *      软锚点的数字不得作为宽容来源，否则改软锚点即可洗白钉句。旧实现把 `hard:false` / `check:"manual"`
 *      锚点的 `value`/`value2`/`planNumber` 一并收进集合，而那些锚点**改值永不判红** ⇒ 攻击链：把
 *      `k1RsFileCount` 的 `value` 与 `planNumber` 同改成盘上实测值（①被同时满足，所以①抓不到），
 *      再把旧数 2005 塞进 manual 锚点 `k1ExternalImplTraitCount`「藏」起来 ⇒ ②条形同虚设，整条钉句
 *      假绿。现口径：只有「登记与盘上实测会真的判红」的锚点才有资格替别人的钉句背书；
 *      `hard` 缺省（未声明 true）同样不给资格 —— fail-closed，宁可少宽容，不可留洗白路径。
 *      现盘仅两条 token 依赖这档宽容，且 donor 都是 hard:true 机器复检锚点 ⇒ 收窄后不新增红：
 *      `commandTotal` 钉句里的 `164` ← `commandModules`（check=registryModuleCount）、
 *      `k1RsFileCount` 钉句里的 `316` ← `k1PubTraitCount`（check=k1PubTraitCount）。
 *   ③ 非纯数字 token（`"28 文件"`、`"A 11 + B 10 + C 5"`）不做数字比对，仍只由 checkPlan 的存在性把关。
 *  分档：本判据是「表自身散架」，与 checkSelfConsistency 同档 ⇒ **一律计入硬失败，不看 anchor.hard**
 *  （否则 hard:false 的 232/51 可以自说自话，判据分档只适用于「登记 vs 实测」那一类差异）。 */
export function checkAnchorConsistency(register) {
  const problems = [];
  const norm = (s) => String(s).replace(/[\s\u00A0\u2009]/g, "");
  const pureNum = (t) => /^\d+(?: \d{3})*$/.test(norm(t));
  const tableNumbers = new Set();
  for (const a of register.anchors) {
    if (a.hard !== true || a.check === "manual") { continue; } // 软锚点/manual 锚点的数不得作宽容来源（见上②）
    for (const v of [a.value, a.value2]) { if (typeof v === "number") { tableNumbers.add(norm(v)); } }
  }
  for (const a of register.anchors) {
    const own = new Set();
    for (const v of [a.value, a.value2]) { if (typeof v === "number") { own.add(norm(v)); } }
    const pins = (a.planPinned ?? []).map((p) => (p.mustContain ?? []).join(" + ")).join(" | ") || "—";
    if (a.planNumber && pureNum(a.planNumber) && norm(a.planNumber) !== norm(a.value)) {
      problems.push(`锚点 ${a.id} 自相矛盾：value=${a.value} / planNumber=${a.planNumber} / mustContain=${pins} —— planNumber 与 value 不是同一个数（①）`);
    }
    for (const pin of a.planPinned ?? []) {
      for (const t of pin.mustContain ?? []) {
        if (!pureNum(t)) { continue; }
        const d = norm(t);
        if (own.has(d) || tableNumbers.has(d)) { continue; }
        problems.push(`锚点 ${a.id} 自相矛盾：value=${a.value} / planNumber=${a.planNumber ?? "—"} / mustContain=${t} —— PLAN:${pin.line} 钉句里的这个数字既非本锚点登记数、也不属于「hard:true 且机器复检」锚点的登记数（②：一侧被改、一侧留旧数；藏进软锚点不算宽容）`);
      }
    }
  }
  return problems;
}

/** 测试替身/产物排除（countRule.implementors：test_support|mock_|/bin/|e2e_|_fixture）。 */
export function isTestDoublePath(rel) {
  const parts = rel.split("/");
  const base = parts[parts.length - 1];
  return base.includes("test_support") || base.includes("mock_") || base.includes("e2e_") || base.includes("_fixture") || parts.includes("bin");
}

/** `^\s*(?:unsafe\s+)?impl … for <trait>` 的机械口径：取 impl 与 for 之间最后一个路径段（剥泛型）。
 *  被实现类型允许以 `$` 开头 —— 宏臂内的 `impl ProviderAdapter for $t`（providers/src/compat.rs:112/198）与
 *  `impl Tool for $name`（tools/src/tools/{browser,finance,obsidian}.rs 共 6 处）**计入站点**：
 *  这正是镜像表 note「实现者 14/199 由宏展开的 impl 站点构成」所声明的同一口径（14=12+2、199=193+6）。 */
const IMPL_LINE_RE = /^\s*(?:unsafe\s+)?impl(?:\s*<[^;{]*?>)?\s+([A-Za-z_][\w:]*(?:<[^>]*>)?)\s+for\s+[A-Za-z_$(]/;
export function implTraitOf(line) {
  const m = IMPL_LINE_RE.exec(line);
  if (!m) { return null; }
  return m[1].replace(/<.*$/, "").split("::").pop();
}

/** seamDeclared（countRule.seamDeclared）：seamId 字面量相等或以 `seamId.` 为前缀。text=null 表示文件缺失。 */
export function seamDeclared(seamId, text) {
  if (text === null || text === undefined) { return null; }
  return text.includes(`"${seamId}"`) || text.includes(`"${seamId}.`);
}

/** 证据锚点判据：文件存在 + 行号存在 + 有 token 时该行（或 [line,endLine] 区间）含该 token。 */
export function adjudicateEvidence(ev, contentFor) {
  const content = contentFor(ev.file);
  if (content === null) { return `登记 ${ev.ref} / 实测 文件不存在`; }
  if (!ev.line) { // fileLevel：只判存在 + token 在文件任一处
    if (ev.token && !content.includes(ev.token)) { return `登记 ${ev.ref}（文件级 token「${ev.token}」）/ 实测 全文无此 token`; }
    return null;
  }
  const lines = content.split("\n");
  if (ev.line > lines.length) { return `登记 ${ev.ref} / 实测 文件仅 ${lines.length} 行（行号越界）`; }
  if (ev.endLine) {
    if (ev.endLine > lines.length) { return `登记 ${ev.ref} 区间止点 ${ev.endLine} / 实测 文件仅 ${lines.length} 行`; }
    const seg = lines.slice(ev.line - 1, ev.endLine);
    if (ev.token && !seg.some((l) => l.includes(ev.token))) { return `登记 token「${ev.token}」应落在 ${ev.line}–${ev.endLine} / 实测 区间内无此 token`; }
    return null;
  }
  const l = lines[ev.line - 1];
  if (ev.token && !l.includes(ev.token)) {
    return `登记 token「${ev.token}」/ 实测 第 ${ev.line} 行「${l.trim().slice(0, 80)}」`;
  }
  return null;
}

/** 数字在文本中的「非行号引用」出现（前后不接数字；左不接 `: . / #`，防 `PLAN:124` 误判）。返回行号列表。 */
export function numberOccurrences(text, num) {
  const hits = [];
  const re = new RegExp(`(^|[^\\d:.#])${num}(?!\\d)`, "g");
  text.split("\n").forEach((l, i) => {
    re.lastIndex = 0;
    if (re.test(l)) { hits.push(i + 1); }
  });
  return hits;
}

/** planNumber 的「数字边界」命中（第 4 条修复 2026-11-02，替代旧的裸子串 `planText.includes(num)`）。
 *  旧实现近乎恒真：短数 `"26"` 在 PLAN 里裸子串命中 52 行（绝大多数是 `2026-11-xx` 日期里的 `026`）、
 *  `"51"` `"83"` 各 11 行 ⇒ 登记值改错了也照样「出现在 PLAN 里」，这一档实际不判任何东西。
 *  新规则（机械，四条，写死在此处）：
 *   ① 取 planNumber 的数字位，**位与位之间允许 0 或 1 个空格**（半角空格 / NBSP / 窄空格），
 *      兼容 PLAN 的千位空格写法 `84 873`、`60 757`；不允许插入任何其它字符；
 *   ② 命中处右侧不得紧邻数字或小数点，左侧不得紧邻数字/小数点/冒号/斜杠/井号
 *      （沿用 numberOccurrences 的「非行号引用」约定：防 `84873.5`、`1.26`、`PLAN:124`、
 *      以及漂移段 `1534/1541`、`124/83` 这类「斜杠后的配对旧数」被当成正读数控数）；
 *   ③ 逐行匹配前先把**日期/时刻上下文整串涂白**（`YYYY-MM-DD`、`YYYY/M/D`、`D.M.YYYY`、`HH:MM(:SS)`
 *      替换为等长空格）⇒ 纯日期行不再贡献命中，日期里的 26/51/83 之类两位数不再假绿；
 *   ④ planNumber 不是纯数字串（若将来出现措辞型锚点）时退回裸子串语义，不放松 ①–③。
 *  返回命中行号列表（空列表 = 未在 PLAN 正文以合法形态出现）。
 *  2026-11-02 现盘核对：15 个 planMustAppear 锚点在 ①–③ 下仍**全部命中**（`26` 由 52 行收敛到 18 行、
 *  `51` 11→2、`83` 11→4、`306` 2→1、`1541` 5、`60 757` 2）⇒ 只削假绿，不新增红。 */
export function planNumberHits(planText, num) {
  const lines = planText.split("\n");
  const digits = String(num).replace(/[\s\u00A0\u2009]/g, "");
  if (!/^\d+$/.test(digits)) {
    return lines.map((l, i) => (l.includes(String(num)) ? i + 1 : -1)).filter((i) => i > 0);
  }
  const dateRes = [
    /\b(?:19|20)\d{2}[-/.]\d{1,2}(?:[-/.]\d{1,2})?\b/g,
    /\b\d{1,2}[-/.](?:19|20)\d{2}\b/g,
    /\b\d{1,2}:\d{2}(?::\d{2})?\b/g,
  ];
  const re = new RegExp(`(?:^|[^\\d.:#/])${digits.split("").join("[ \\u00A0\\u2009]?")}(?![\\d.])`, "g");
  const hits = [];
  lines.forEach((raw, i) => {
    let l = raw;
    for (const dr of dateRes) { l = l.replace(dr, (m) => " ".repeat(m.length)); }
    re.lastIndex = 0;
    if (re.test(l)) { hits.push(i + 1); }
  });
  return hits;
}

/** §0.12.4 漂移列举段区段（判定规则见文件头注释 —— 区段含「漂移」字样才作数）。 */
export function driftRegionLines(planText) {
  const lines = planText.split("\n");
  const start = lines.findIndex((l) => /^#{1,4}\s.*0\.12\.4/.test(l));
  if (start < 0) { return null; }
  const level = (lines[start].match(/^#+/) || [""])[0].length;
  let end = lines.length;
  for (let i = start + 1; i < lines.length; i++) {
    const m = /^(#+)\s/.exec(lines[i]);
    if (m && m[1].length <= level) { end = i; break; }
  }
  const region = lines.slice(start, end).join("\n");
  if (!region.includes("漂移")) { return null; }
  return { start: start + 1, end };
}

/** PLAN 正文对账（只读 docs/**，绝不写）：planMustAppear / planPinned / planOld / bannedPlanPhrases。
 *  分级与锚点自身一致：hard:false 锚点（如 manual 档 232/51）的 planNumber/钉句缺失进 soft（只报告），
 *  与「manual ⇒ 不判红」的总分档一致；禁语属 caliber 级判据，一律 hard。 */
export function checkPlan(register, planText) {
  const fails = [];
  const soft = [];
  const passed = [];
  if (planText === null) {
    fails.push(`PLAN 文件缺失（${register.sources.plan}）—— 跳过 ≠ 通过：planMustAppear / planPinned / planOld / bannedPhrases 全部无法核对`);
    return { fails, soft, passed };
  }
  const lines = planText.split("\n");
  let appears = 0;
  for (const a of register.anchors) {
    const sink = a.hard === false ? soft : fails;
    if (a.planMustAppear && a.planNumber) {
      // 第 4 条修复：数字边界命中（旧 `planText.includes` 裸子串对短数近乎恒真，规则见 planNumberHits 注释）
      const hits = planNumberHits(planText, a.planNumber);
      if (!hits.length) {
        sink.push(`锚点 ${a.id}：planNumber「${a.planNumber}」未在 PLAN 正文以数字边界形态命中（裸子串旧口径已废 —— 只允许日期涂白后、前后不接数字/小数点的出现算数）`);
      } else { appears++; }
    }
    for (const pin of a.planPinned ?? []) {
      const l = lines[pin.line - 1];
      const missing = pin.mustContain.filter((t) => !(l ?? "").includes(t));
      if (l === undefined || missing.length) {
        sink.push(`锚点 ${a.id} 钉句 PLAN:${pin.line} 不符 —— 缺「${missing.join(" / ") || (l === undefined ? "该行不存在" : "")}」`);
      }
    }
    for (const old of a.planOld ?? []) {
      const region = driftRegionLines(planText);
      const hits = numberOccurrences(planText, old.number);
      for (const ln of hits) {
        if (!region || ln < region.start || ln > region.end) {
          sink.push(`旧读数 ${old.number} 出现在漂移列举段之外（PLAN:${ln}${region ? `，合法区段仅 ${region.start}–${region.end}` : "，且找不到含「漂移」的 §0.12.4 区段"}）—— ${old.note}`);
        }
      }
    }
  }
  for (const ph of register.caliber.bannedPlanPhrases ?? []) {
    if (planText.includes(ph)) { fails.push(`并行口径禁语「${ph}」出现在 PLAN 正文（定案口径见 caliber.statement）`); }
  }
  passed.push(`PLAN 对账：planMustAppear ${appears} 项 · planPinned ${register.anchors.reduce((n, a) => n + (a.planPinned?.length ?? 0), 0)} 处 · planOld ${register.anchors.reduce((n, a) => n + (a.planOld?.length ?? 0), 0)} 项 · 禁语 ${(register.caliber.bannedPlanPhrases ?? []).length} 条`);
  return { fails, soft, passed };
}

/** 总对账：读表 + 读数 + PLAN 文本 ⇒ 硬失败/报告/未机器复检三本账。纯函数（内容经 readings.contentFor 注入）。 */
export function reconcile(register, readings, planText) {
  const hardFails = [];
  const reports = [];
  const manualLines = [];
  const anchorResults = [];
  const entryResults = [];

  // ① 表自身一致性（镜像表是判据 ⇒ 它自己散架必红）
  for (const p of checkSelfConsistency(register)) { hardFails.push(`表一致性：${p}`); }
  // ①b 锚点内部自洽（第 1 条修复：value / planNumber / planPinned.mustContain 三对齐；不看 anchor.hard）
  const anchorConsistency = checkAnchorConsistency(register);
  for (const p of anchorConsistency) { hardFails.push(p); }

  const remoteSet = new Set(readings.remoteSeams ?? []);
  // 封闭集合对账（countRule.closure）：13 = A 11 + K 组 agent.loop / session.log.invariant，多一条少一条都红
  const expectRemote = register.entries.filter((e) => e.expect && e.expect.remote === true).map((e) => e.seamId).sort();
  const seamAnchor = register.anchors.find((a) => a.check === "remoteSeamRegistryCount");
  // 计数/封闭两本账先算好（消息里的 seamId 由锚点行统一印出，见下方 switch 的 remoteSeamRegistryCount 分支）
  let remoteCountIssue = null;
  let remoteClosureIssue = null;
  if (readings.remoteSeams !== null) {
    const actual = [...remoteSet].sort();
    const expectSet = new Set(expectRemote);
    const missing = expectRemote.filter((s) => !remoteSet.has(s));
    const extra = actual.filter((s) => !expectSet.has(s));
    if (seamAnchor && readings.remoteSeams.length !== seamAnchor.value) {
      remoteCountIssue = `登记 ${seamAnchor.value} 条 / 实测 ${readings.remoteSeams.length} 条 / 差 ${readings.remoteSeams.length - seamAnchor.value}`;
    }
    if (missing.length || extra.length) {
      remoteClosureIssue = `封闭集合破裂：缺 [${missing.join(", ")}] / 多 [${extra.join(", ")}]（登记面=表中 remote:true 的 ${expectRemote.length} 条）`;
    }
  }

  // ② 锚点
  for (const a of register.anchors) {
    let mismatch = null;
    const cmpNum = (actual, exp, unit) => {
      if (actual === undefined || actual === null) { return "实测读数缺失（源文件未见）"; }
      if (actual !== exp) { return `登记 ${exp}${unit ?? ""} / 实测 ${actual}${unit ?? ""} / 差 ${actual - exp}`; }
      return null;
    };
    switch (a.check) {
      case "registryCommandCount":
        mismatch = !readings.registry ? "注册表缺失 —— 跳过 ≠ 通过" : cmpNum(readings.registry.commandTotal, a.value, " 条");
        break;
      case "registryModuleCount":
        mismatch = !readings.registry ? "注册表缺失 —— 跳过 ≠ 通过" : cmpNum(readings.registry.commandModules, a.value, " 模块");
        break;
      case "registryUnassigned":
        mismatch = !readings.registry ? "注册表缺失 —— 跳过 ≠ 通过"
          : readings.registry.unassigned.length ? `未归属模块非空（domainOf 兜底「其它」）：${readings.registry.unassigned.join(", ")} —— 非空即漂移，PLAN:226 候选集前提失效` : null;
        break;
      case "registryFileLineCount":
        mismatch = !readings.registry ? null : cmpNum(readings.registry.registryFileLines, a.value, " 行");
        break;
      case "registryBoundaryCount": {
        const r = readings.boundary?.[a.domainKey];
        mismatch = !r ? "读数缺失" : (cmpNum(r.total, a.value, " 条") ?? cmpNum(r.modules, a.value2, ` ${a.value2Label}`));
        break;
      }
      case "registryDomainCount": {
        const r = readings.auditDomain?.[a.domainName];
        mismatch = !r ? `域「${a.domainName}」不存在于 domainOf 读数（域名或 DOMAIN_RULES 变了 ⇒ 同批改表）`
          : (cmpNum(r.total, a.value, " 条") ?? cmpNum(r.modules, a.value2, ` ${a.value2Label}`));
        break;
      }
      case "frontendVolume":
        mismatch = !readings.frontend ? "前端读数缺失" : (cmpNum(readings.frontend.lines, a.value, " 行") ?? cmpNum(readings.frontend.files, a.value2, " 文件"));
        break;
      case "frontendDirVolume": {
        const r = readings.frontendDir?.[a.dir];
        mismatch = !r ? `目录 ${a.dir} 读数缺失` : (cmpNum(r.lines, a.value, " 行") ?? cmpNum(r.files, a.value2, " 文件"));
        break;
      }
      case "l1ForeignBlockers": {
        const r = readings.l1?.[a.domainKey];
        mismatch = !r ? "L1 读数缺失" : (cmpNum(r.hits, a.value, " 处") ?? cmpNum(r.files, a.value2, " 文件"));
        break;
      }
      case "k1RsFileCount":
        mismatch = !readings.k1 ? "K1 扫描面缺失" : cmpNum(readings.k1.rsFiles, a.value, " 文件");
        break;
      case "k1PubTraitCount":
        mismatch = !readings.k1 ? "K1 扫描面缺失" : cmpNum(readings.k1.pubTraits, a.value, " trait");
        break;
      case "remoteSeamRegistryCount":
        mismatch = readings.remoteSeams === null ? "worker.rs 缺失 —— 跳过 ≠ 通过（数量与封闭集合均无法核对）"
          : (remoteCountIssue ?? remoteClosureIssue);
        break;
      case "registerGroupSum":
        mismatch = null; // 已在表一致性处判
        break;
      case "manual":
        reports.push(`[只报告] 锚点 ${a.id}（${a.label}）：登记 ${a.value}${a.implementorsMechanical !== undefined && readings.k1 ? ` / 本门机械复算 ${readings.k1.externalImplTraits}（表内自记 ${a.implementorsMechanical}）` : ""} —— ${a.reason ?? "不可机械复算"}`);
        break;
      default:
        mismatch = `未知 check「${a.check}」`;
    }
    if (a.check !== "manual") {
      anchorResults.push({
        status: !mismatch ? "PASS" : a.hard === false ? "REPORT" : "FAIL",
        txt: `${a.id}（${a.label}）= ${a.value}${mismatch ? ` —— ${mismatch}` : ""}`,
      });
    }
    if (mismatch && a.check !== "manual") {
      const line = `锚点 ${a.id}（${a.label}）：${mismatch}`;
      if (a.hard === false) { reports.push(`[只报告] ${line}`); } else { hardFails.push(`FAIL: ${line}`); }
    }
  }
  // 锚点自洽档的通过面也印出来（否则「绿」只显示逐锚点读数，看不出这条新判据到底跑没跑）
  anchorResults.push({
    status: anchorConsistency.length ? "FAIL" : "PASS",
    txt: `内部自洽（value/planNumber/mustContain 三对齐）：${register.anchors.length} 项${anchorConsistency.length ? ` —— ${anchorConsistency.length} 项矛盾` : ""}`,
  });

  // ③ 逐条登记项
  let machineRows = 0;
  let manualRows = 0;
  for (const e of register.entries) {
    const probs = [];
    const loc = `${e.seamId}（组 ${e.group} · planLine ${e.planLine}）`;
    for (const ev of e.evidence ?? []) {
      const p = adjudicateEvidence(ev, readings.contentFor);
      if (p) { probs.push(`证据 ${ev.ref}：${p}`); }
    }
    if (e.check === "manual") {
      manualRows++;
      manualLines.push(`${loc} —— 未机器复检：${e.reason ?? "原文无可机械复检判据"}${e.expectMissingFields?.length ? `（缺省字段：${e.expectMissingFields.join(" / ")}）` : ""}`);
      continue;
    }
    machineRows++;
    const faces = e.faces ?? [e.seamId];
    if (e.check === "implementorCount" && e.trait) {
      const expected = e.expect.implementors ?? (e.expectImplementorZero ? 0 : null);
      if (expected !== null && e.implementorsHard !== false) {
        if (!readings.implementors) { probs.push(`实现者读数缺失（扫描面未产出，trait=${e.trait}）`); }
        // 站点计数里「键不存在」= 全扫描面零命中 = 实测 0（context.engine / rule.custom / dojo 的 0 值判据正是此形态）
        else {
          const actual = readings.implementors[e.trait] ?? 0;
          if (actual !== expected) { probs.push(`实现者「${e.trait}」：登记 ${expected} / 实测 ${actual} / 差 ${actual - expected}`); }
        }
      }
    }
    const state = e.expect.seam ?? "";
    if (/^(已建|待建|有接缝)/.test(state)) {
      for (const f of faces) {
        const declared = seamDeclared(f, readings.seamRegistryText);
        if (declared === null) { probs.push(`capability_registry.rs 缺失 —— 接缝「${f}」无法核对（跳过 ≠ 通过）`); break; }
        if (state.startsWith("待建") && declared) { probs.push(`接缝「${f}」登记「${state}」/ 实测已在 capability_registry.rs 登记`); }
        if ((state.startsWith("已建") || state.startsWith("有接缝")) && !declared) { probs.push(`接缝「${f}」登记「${state}」/ 实测 capability_registry.rs 无字面量`); }
      }
    }
    if (e.expect.remote === true && !remoteSet.has(e.seamId)) { probs.push("期望可远程 / 实测不在 SUPPORTED_REMOTE_SEAMS"); }
    if (e.expect.remote === false && remoteSet.has(e.seamId)) { probs.push("期望不可远程 / 实测在 SUPPORTED_REMOTE_SEAMS"); }
    if (e.implementorsMechanical !== undefined) {
      reports.push(`[只报告] ${loc}：实现者机械读数 ${e.implementorsMechanical}${e.expect.implementors === null ? "（行内登记值缺省，无登记值可比）" : ` / 登记 ${e.expect.implementors}`} —— ${e.reason ?? "口径不可比"}`);
    }
    if (e.expectMissing) {
      reports.push(`[字段缺省] ${loc}：${(e.expectMissingFields ?? []).join(" / ")} —— 缺省字段不算通过（note：字段缺省=PLAN 原文就缺）`);
    }
    if (probs.length) { hardFails.push(`FAIL: 登记项 ${loc} —— ${probs.join("；")}`); }
    entryResults.push({ ok: !probs.length, txt: loc });
  }

  // ④ PLAN 正文对账
  const plan = checkPlan(register, planText);
  for (const f of plan.fails) { hardFails.push(`FAIL: ${f}`); }
  for (const s of plan.soft) { reports.push(`[只报告] ${s}`); }

  return { hardFails, reports, manualLines, anchorResults, entryResults, planPassed: plan.passed, counts: { total: register.entries.length, machine: machineRows, report: reports.length, manual: manualRows } };
}

// ── --repin-plan（任务 #48）：按定位键把「PLAN 行号引用」重锚到当前文本 ──────────
// 判据与红线见文件头「--repin-plan 的由来与判据」段。以下四个是**纯函数**（selftest 与正式跑共用同一实现）：
// 定位（pinHitLines / textHitLines）⇒ 汇总（collectPlanLineRefs）⇒ 计算（computeRepin，只给方案不写盘）
// ⇒ 回写（applyRepinToRegister 内存版 + planRepinPatches 文本版）。

/** 整行归一：去首尾空白、行内连续空白折叠成一个空格 —— 只容排版差异，不容内容差异。 */
export function normPlanLine(s) {
  return String(s ?? "").trim().replace(/[\t ]+/g, " ");
}

/** 钉句候选行：mustContain 的**全部** token 落在同一行才算命中（与 checkPlan 同一判据形态）。 */
export function pinHitLines(planLines, mustContain) {
  const toks = (mustContain ?? []).filter((t) => String(t).length > 0);
  if (!toks.length) { return []; }
  return planLines.map((l, i) => (toks.every((t) => l.includes(t)) ? i + 1 : 0)).filter(Boolean);
}

/** 指针候选行：整行逐字（归一后）全等才算命中。 */
export function textHitLines(planLines, anchorText) {
  const t = normPlanLine(anchorText);
  if (!t) { return []; }
  return planLines.map((l, i) => (normPlanLine(l) === t ? i + 1 : 0)).filter(Boolean);
}

/** 全部「受行号约束」的 PLAN 引用（三类），各带自己的定位键与写回坐标。 */
export function collectPlanLineRefs(register) {
  const refs = [];
  for (const a of register.anchors ?? []) {
    (a.planPinned ?? []).forEach((p, i) => {
      refs.push({ kind: "planPinned", owner: a.id, idx: i, line: p.line, mustContain: p.mustContain ?? [], anchorText: null, path: `anchors[${a.id}].planPinned[${i}].line` });
    });
  }
  for (const e of register.entries ?? []) {
    if (e.planLine !== undefined && e.planLine !== null) {
      refs.push({ kind: "planLine", owner: e.seamId, idx: null, line: e.planLine, mustContain: null, anchorText: e.planAnchorText ?? null, path: `entries[${e.seamId}].planLine` });
    }
    (e.planLineRange ?? []).forEach((n, i) => {
      refs.push({ kind: "planLineRange", owner: e.seamId, idx: i, line: n, mustContain: null, anchorText: (e.planAnchorTextRange ?? [])[i] ?? null, path: `entries[${e.seamId}].planLineRange[${i}]` });
    });
  }
  return refs;
}

/** 计算重锚方案（零写盘）。changes=可回写项 · blockers=歧义/零命中（整次拒绝）· noKey=缺定位键（同拒）。 */
export function computeRepin(register, planText) {
  const planLines = String(planText ?? "").split("\n");
  const changes = [];
  const blockers = [];
  const noKey = [];
  let kept = 0;
  const refs = collectPlanLineRefs(register);
  for (const r of refs) {
    const cur = planLines[r.line - 1] ?? "";
    let curOk;
    let hits;
    if (r.kind === "planPinned") {
      if (!r.mustContain.length) { noKey.push(`${r.path} = ${r.line}：无 mustContain token ⇒ 无定位键`); continue; }
      curOk = r.mustContain.every((t) => cur.includes(t));
      hits = curOk ? [r.line] : pinHitLines(planLines, r.mustContain);
    } else {
      if (!r.anchorText) { noKey.push(`${r.path} = ${r.line}：无 planAnchorText 定位键（新增行号引用必须同批带键，否则重锚能力随表腐烂）`); continue; }
      curOk = normPlanLine(cur) === normPlanLine(r.anchorText);
      hits = curOk ? [r.line] : textHitLines(planLines, r.anchorText);
    }
    if (curOk) { kept++; continue; }
    if (hits.length === 1) { changes.push({ ...r, to: hits[0] }); continue; }
    if (hits.length === 0) {
      blockers.push(`${r.path} = ${r.line}：定位键在 PLAN 内**零命中**（该句已被改写或删除）⇒ 不许猜行、不许插值，请先人工核对该项`);
      continue;
    }
    blockers.push(`${r.path} = ${r.line}：定位键**非唯一命中** ${hits.length} 行 [${hits.join(", ")}] ⇒ 请先收紧 mustContain / 用更长的整行文本，不许从里面挑一个`);
  }
  return { changes, blockers, noKey, kept, total: refs.length };
}

/** 内存版回写（也是文本版补丁的对照物：两者必须逐字段一致才允许落盘）。 */
export function applyRepinToRegister(register, changes) {
  const next = JSON.parse(JSON.stringify(register));
  for (const c of changes) {
    if (c.kind === "planPinned") {
      const a = next.anchors.find((x) => x.id === c.owner);
      a.planPinned[c.idx].line = c.to;
    } else {
      const e = next.entries.find((x) => x.seamId === c.owner);
      if (c.kind === "planLine") { e.planLine = c.to; } else { e.planLineRange[c.idx] = c.to; }
    }
  }
  return next;
}

function countOcc(hay, needle) {
  let n = 0;
  let i = 0;
  while ((i = hay.indexOf(needle, i)) >= 0) { n++; i += needle.length; }
  return n;
}

const rangeLiteral = (arr) => `[${arr.join(", ")}]`;
const rangeLiteralPretty = (arr) => `[\n        ${arr.join(",\n        ")}\n      ]`;

/**
 * 文本版回写：对每条 change 用「候选 needle 里第一个在全文**恰好出现一次**的」做定点替换
 * （不整体重新序列化 ⇒ 未涉及的字节逐字不动，与并行编辑者互不打散格式）。
 * 任一 change 找不到唯一 needle ⇒ {ok:false, problems}，调用方**不得写盘**。
 * `planLineRange` 的两个端点合并按整个数组字面量替换（否则改完端点 0 后端点 1 的原文就变了）。
 */
export function planRepinPatches(srcText, register, changes) {
  const problems = [];
  const patches = [];
  const groups = new Map();
  for (const c of changes) {
    const key = c.kind === "planLineRange" ? `planLineRange|${c.owner}` : `${c.kind}|${c.owner}|${c.idx}`;
    if (!groups.has(key)) { groups.set(key, []); }
    groups.get(key).push(c);
  }
  const lineNumRe = (n) => new RegExp(`"line": ${n}(?!\\d)`, "g");
  const planLineRe = (n) => new RegExp(`"planLine": ${n}(?!\\d)`, "g");
  for (const [key, list] of groups) {
    const kind = key.split("|")[0];
    const owner = list[0].owner;
    let needles;
    let sub;
    if (kind === "planPinned") {
      const c = list[0];
      const items = c.mustContain.map((t) => JSON.stringify(t)).join(", ");
      needles = [`{ "line": ${c.line}, "mustContain": [${items}] }`, `"line": ${c.line}, "mustContain": [${items}]`];
      sub = { re: lineNumRe(c.line), to: `"line": ${c.to}` };
    } else if (kind === "planLine") {
      const c = list[0];
      const at = JSON.stringify(c.anchorText);
      needles = [`"planAnchorText": ${at}, "planLine": ${c.line}`, `"planAnchorText": ${at},\n      "planLine": ${c.line}`, `"planLine": ${c.line}`];
      sub = { re: planLineRe(c.line), to: `"planLine": ${c.to}` };
    } else {
      const e = (register.entries ?? []).find((x) => x.seamId === owner);
      const oldArr = [...(e?.planLineRange ?? [])];
      const newArr = oldArr.map((n, i) => list.find((c) => c.idx === i)?.to ?? n);
      if (!list.length || !oldArr.length) { problems.push(`${owner} planLineRange：登记表里没有该区间，无法定点替换`); continue; }
      if (rangeLiteral(oldArr) === rangeLiteral(newArr)) { continue; }
      needles = [`"planLineRange": ${rangeLiteral(oldArr)}`, `"planLineRange": ${rangeLiteralPretty(oldArr)}`];
      sub = { literal: rangeLiteral(oldArr), to: rangeLiteral(newArr), literalPretty: rangeLiteralPretty(oldArr), toPretty: rangeLiteralPretty(newArr) };
    }
    const hit = needles.find((n) => countOcc(srcText, n) === 1);
    if (!hit) {
      problems.push(`${list.map((c) => `${c.path} ${c.line}→${c.to}`).join(" / ")}：镜像表原文里找不到唯一的替换锚（排版被重排过、或该引用不止一处）⇒ 拒绝定点写盘，请人工改表`);
      continue;
    }
    let replacement;
    if (sub.literal !== undefined) { replacement = hit.includes(sub.literal) ? hit.replace(sub.literal, sub.to) : hit.replace(sub.literalPretty, sub.toPretty); } else { replacement = hit.replace(sub.re, sub.to); }
    if (replacement === hit) { problems.push(`${list.map((c) => c.path).join(" / ")}：替换锚命中但行号没被改动（判据落空）⇒ 拒绝写盘`); continue; }
    patches.push({ needle: hit, replacement, paths: list.map((c) => c.path) });
  }
  if (problems.length) { return { ok: false, problems, patches: [], text: null }; }
  let out = srcText;
  for (const p of patches) {
    const at = out.indexOf(p.needle);
    out = `${out.slice(0, at)}${p.replacement}${out.slice(at + p.needle.length)}`;
  }
  let parsed;
  try {
    parsed = JSON.parse(out);
  } catch (e) {
    return { ok: false, problems: [`定点替换后 JSON 解析失败（说明替换落到了字符串字面量里）：${e.message}`], patches, text: null };
  }
  const expect = applyRepinToRegister(register, changes);
  if (JSON.stringify(parsed) !== JSON.stringify(expect)) {
    return { ok: false, problems: ["文本版回写与内存版回写结果不一致 ⇒ 定点替换的把握不足，拒绝写盘（宁可人工改表）"], patches, text: null };
  }
  return { ok: true, text: out, problems: [], patches };
}

/** 只保留「非行号」字段 —— 用于断言重锚**只改行号**，判据（token/读数/分组）一个字都没松。 */
export function stripRepinEditableFields(register) {
  const clone = JSON.parse(JSON.stringify(register));
  for (const a of clone.anchors ?? []) { for (const p of a.planPinned ?? []) { p.line = null; } }
  for (const e of clone.entries ?? []) {
    if (e.planLine !== undefined) { e.planLine = null; }
    if (e.planLineRange) { e.planLineRange = e.planLineRange.map(() => null); }
  }
  return clone;
}

// ── selftest（纯函数、零盘；负控+正控成对）──────────────────────────────
function fixture() {
  const register = {
    note: "fixture",
    caliber: { capacityRegister: 2, groupCounts: { A: 1, B: 1, C: 0, H: 0, K: 1 }, totalRows: 3, bannedPlanPhrases: ["B 11", "登记 27"] },
    sources: { plan: "docs/PLAN-x.md" },
    countRule: {},
    // 每种 anchor check 各配一个锚点（与真实表同档：registryFileLineCount 在真实表是 hard:false）⇒
    // 干净读数（正控）与逐类改读数（负控）都能在同一张 fixture 上成对跑，见 ANCHOR_CHECK_NEGATIVE_CASES
    anchors: [
      { id: "commandTotal", label: "命令总数", value: 10, check: "registryCommandCount", hard: true, planNumber: "10", planMustAppear: true, planPinned: [{ line: 1, mustContain: ["10", "3"] }], planOld: [{ number: "9", note: "旧读数，只许在漂移段" }] },
      { id: "commandModules", label: "命令模块数", value: 3, check: "registryModuleCount", hard: true },
      { id: "unassignedModules", label: "未归属", value: 0, check: "registryUnassigned", hard: true },
      { id: "registryFileLines", label: "注册表行数", value: 5, check: "registryFileLineCount", hard: false },
      { id: "stockBoundary", label: "股票边界命令", value: 4, value2: 2, value2Label: "模块", check: "registryBoundaryCount", domainKey: "STOCK", hard: true },
      { id: "stockDomain", label: "股票域命令", value: 4, value2: 2, value2Label: "模块", check: "registryDomainCount", domainName: "股票域", hard: true },
      { id: "frontendVolume", label: "前端体量", value: 100, value2: 4, value2Label: "文件", check: "frontendVolume", hard: true },
      { id: "frontendDirVolume", label: "前端目录体量", value: 40, value2: 2, value2Label: "文件", check: "frontendDirVolume", dir: "src/x", hard: true },
      { id: "l1Stock", label: "L1 股票阻塞", value: 6, value2: 2, value2Label: "文件", check: "l1ForeignBlockers", domainKey: "STOCK", hard: true },
      { id: "k1Files", label: "K1 .rs 文件数", value: 12, check: "k1RsFileCount", hard: true },
      { id: "k1Traits", label: "K1 pub trait 总数", value: 4, check: "k1PubTraitCount", hard: true },
      { id: "remoteSeamCount", label: "远程清单", value: 1, check: "remoteSeamRegistryCount", hard: true },
      { id: "capacityRegister", label: "能力面", value: 2, check: "registerGroupSum", hard: true },
      { id: "k1Ext", label: "外部实现者", value: 99, check: "manual", hard: false, implementorsMechanical: 7 },
    ],
    entries: [
      { seamId: "a.one", group: "A", trait: "TraitOne", expect: { implementors: 2, seam: "已建", remote: true }, check: "implementorCount", planAnchorText: "登记项一 a.one 已建。", planLine: 5, evidence: [{ ref: "f.rs:3", file: "f.rs", line: 3, token: "pub trait TraitOne" }] },
      { seamId: "b.two", group: "B", trait: null, expect: { implementors: null, seam: "待建", remote: false }, expectMissing: true, expectMissingFields: ["implementors"], check: "seamExists", faces: ["b.two"], planAnchorText: "登记项二 b.two 待建。", planLine: 6, evidence: [], planLineRange: [5, 6], planAnchorTextRange: ["登记项一 a.one 已建。", "登记项二 b.two 待建。"] },
      { seamId: "k.three", group: "K", expect: { implementors: null, seam: null, remote: null }, check: "manual", reason: "无可机械复检判据", planAnchorText: "登记项三 k.three 人工。", planLine: 7, evidence: [] },
    ],
    pipeline: {},
  };
  const readings = {
    registry: { commandTotal: 10, commandModules: 3, unassigned: [], registryFileLines: 5 },
    boundary: { STOCK: { total: 4, modules: 2 } },
    auditDomain: { 股票域: { total: 4, modules: 2 } },
    frontend: { lines: 100, files: 4 },
    frontendDir: { "src/x": { lines: 40, files: 2 } },
    l1: { STOCK: { hits: 6, files: 2 } },
    k1: { rsFiles: 12, pubTraits: 4, externalImplTraits: 7 },
    implementors: { TraitOne: 2 },
    remoteSeams: ["a.one"],
    seamRegistryText: 'const X = "a.one";',
    contentFor: (p) => (p === "f.rs" ? "ln1\nln2\npub trait TraitOne {" : null),
  };
  // 5/6/7 三行是「登记项 planLine 的落点」，供 --repin-plan 的正负对照使用（见 ㉑ 系列）。
  const plan = "前言：能力面 10 条 / 3 模块。\n\n### 0.12.4 漂移列举\n本轮抓出 9 → 10 的数字漂移。\n登记项一 a.one 已建。\n登记项二 b.two 待建。\n登记项三 k.three 人工。\n\n### 0.13 下一节\n别的事。\n";
  return { register, readings, plan };
}

// 逐类 anchor check 的负控清单（第 2 条修复）：每一类都配「登记 ≠ 实测 ⇒ 必须被抓出」的合成读数改造。
// 正控 = 同一 fixture 的干净读数下该锚点必须 PASS/REPORT 且不进 hardFails（见下方正向断言）。
// tier: "hard" ⇒ 差异进 hardFails；"report" ⇒ 该锚点在真实表是 hard:false，差异只许进「只报告」（不许静默绿）。
const ANCHOR_CHECK_NEGATIVE_CASES = [
  { check: "registryCommandCount", id: "commandTotal", tier: "hard", mutate: (f) => { f.readings.registry.commandTotal = 11; } },
  { check: "registryModuleCount", id: "commandModules", tier: "hard", mutate: (f) => { f.readings.registry.commandModules = 4; } },
  { check: "registryUnassigned", id: "unassignedModules", tier: "hard", mutate: (f) => { f.readings.registry.unassigned = ["zz_orphan"]; } },
  { check: "registryFileLineCount", id: "registryFileLines", tier: "report", mutate: (f) => { f.readings.registry.registryFileLines = 6; } },
  { check: "registryBoundaryCount", id: "stockBoundary", tier: "hard", mutate: (f) => { f.readings.boundary.STOCK.total = 5; } },
  { check: "registryBoundaryCount", id: "stockBoundary", tier: "hard", mutate: (f) => { f.readings.boundary.STOCK.modules = 3; } },
  { check: "registryDomainCount", id: "stockDomain", tier: "hard", mutate: (f) => { delete f.readings.auditDomain["股票域"]; } },
  { check: "registryDomainCount", id: "stockDomain", tier: "hard", mutate: (f) => { f.readings.auditDomain["股票域"].modules = 5; } },
  { check: "frontendVolume", id: "frontendVolume", tier: "hard", mutate: (f) => { f.readings.frontend.lines = 101; } },
  { check: "frontendVolume", id: "frontendVolume", tier: "hard", mutate: (f) => { f.readings.frontend.files = 5; } },
  { check: "frontendDirVolume", id: "frontendDirVolume", tier: "hard", mutate: (f) => { f.readings.frontendDir["src/x"].files = 3; } },
  { check: "frontendDirVolume", id: "frontendDirVolume", tier: "hard", mutate: (f) => { delete f.readings.frontendDir["src/x"]; } },
  { check: "l1ForeignBlockers", id: "l1Stock", tier: "hard", mutate: (f) => { f.readings.l1.STOCK.hits = 7; } },
  { check: "l1ForeignBlockers", id: "l1Stock", tier: "hard", mutate: (f) => { f.readings.l1.STOCK.files = 3; } },
  { check: "k1RsFileCount", id: "k1Files", tier: "hard", mutate: (f) => { f.readings.k1.rsFiles = 13; } },
  { check: "k1PubTraitCount", id: "k1Traits", tier: "hard", mutate: (f) => { f.readings.k1.pubTraits = 5; } },
  { check: "k1RsFileCount", id: "k1Files", tier: "hard", mutate: (f) => { f.readings.k1 = null; } },
  { check: "remoteSeamRegistryCount", id: "remoteSeamCount", tier: "hard", mutate: (f) => { f.readings.remoteSeams = ["a.one", "x.extra"]; } },
  { check: "registerGroupSum", id: "capacityRegister", tier: "hard", mutate: (f) => { f.register.anchors.find((a) => a.id === "capacityRegister").value = 3; } },
  { check: "manual", id: "k1Ext", tier: "report", mutate: (f) => { f.readings.k1.externalImplTraits = 99; } },
];

function runSelftest() {
  let fail = 0;
  const check = (cond, msg) => {
    if (!cond) { console.error(`SELFTEST FAIL：${msg}`); fail++; }
  };
  const clone = (x) => JSON.parse(JSON.stringify(x));
  const has = (r, re) => r.hardFails.some((f) => re.test(f));
  const anchor = (f, id) => f.register.anchors.find((a) => a.id === id);

  // 正控：干净读数不得假阳性
  const f0 = fixture();
  let r = reconcile(f0.register, f0.readings, f0.plan);
  const cleanResults = r.anchorResults;
  check(r.hardFails.length === 0, `正控：干净读数应零硬失败，实得 ${r.hardFails.join(" | ")}`);
  check(r.counts.machine === 2 && r.counts.manual === 1, "正控：机器复检 2 / 未机器复检 1");

  // 负控①：命令数被改
  const f1 = fixture(); f1.readings.registry.commandTotal = 1534;
  r = reconcile(f1.register, f1.readings, f1.plan);
  check(has(r, /commandTotal/), "负控：命令总数被改必须红");

  // 负控②：分组计数漂移（B 10→B 11 的 fixture 版：B 1→2 ⇒ Σ=3≠2 且行数不符）
  const f2 = fixture(); f2.register.caliber.groupCounts.B = 2;
  const probs = checkSelfConsistency(f2.register);
  check(probs.some((p) => /分组 B/.test(p)) && probs.some((p) => /Σ\(A\+B\+C\)/.test(p)), "负控：分组计数漂移必须报分组与 Σ 两处");
  check(checkSelfConsistency(f0.register).length === 0, "正控：干净表无一致性问题");

  // 负控③：evidence 行号漂移
  const f3 = fixture(); f3.register.entries[0].evidence[0].line = 2;
  r = reconcile(f3.register, f3.readings, f3.plan);
  check(has(r, /a\.one.*实测 第 2 行/), "负控：证据行号漂移必须红并点名 seamId");

  // 负控④：未归属模块非空
  const f4 = fixture(); f4.readings.registry.unassigned = ["zz_orphan"];
  r = reconcile(f4.register, f4.readings, f4.plan);
  check(has(r, /unassignedModules/), "负控：未归属非空必须红");

  // 负控⑤：旧读数出现在漂移段之外
  const f5 = fixture();
  r = reconcile(f5.register, f5.readings, "正文里出现旧读数 9 条（无任何漂移字样）。\n");
  check(has(r, /旧读数 9/), "负控：旧读数在漂移段外必须红");
  // 正控：旧读数在含「漂移」的 §0.12.4 区段内 ⇒ 允许（f0 已含）
  const f5b = fixture();
  r = reconcile(f5b.register, f5b.readings, f5b.plan.replace("### 0.13 下一节", "### 0.13 下一节\n这里再写一次 9 条就违规了"));
  check(has(r, /旧读数 9 出现在漂移列举段之外/), "负控⑤b：漂移段之后的同节外出现也要红");

  // 负控⑥：bannedPlanPhrases 命中
  const f6 = fixture();
  r = reconcile(f6.register, f6.readings, f6.plan + "\n某段写了 B 11。\n");
  check(has(r, /并行口径禁语/), "负控：禁语命中必须红");

  // 负控⑦：manual 行不得被当成通过
  r = reconcile(f0.register, f0.readings, f0.plan);
  check(r.manualLines.some((m) => m.includes("k.three") && m.includes("未机器复检")), "负控：manual 行必须进「未机器复检」摘要");
  const f7 = fixture(); f7.register.entries[2].expect.implementors = 777; // manual 行登记值再离谱也不许进 hardFails
  r = reconcile(f7.register, f7.readings, f7.plan);
  // 第 3 条修复：counts.machine 也必须钉死 —— 若 `continue` 分支被删，manual 行会被误计进机器复检，
  // 单看 hardFails===0 && manual===1 测不出来（3 行表里 manual 既是 manual 又是 machine 才叫假绿）。
  check(r.hardFails.length === 0 && r.counts.manual === 1 && r.counts.machine === 2, `负控⑦b：manual 行不判红、只计 1 条未机器复检且**不占机器复检名额**（实得 manual=${r.counts.manual} machine=${r.counts.machine}）`);

  // 负控⑧：接缝状态（待建却在 registry 出现 / 已建却缺失）与远程清单封闭集合
  const f8 = fixture(); f8.readings.seamRegistryText = '"b.two"';
  r = reconcile(f8.register, f8.readings, f8.plan);
  check(has(r, /接缝「b\.two」登记「待建」/), "负控：待建接缝已被登记必须红");
  const f8b = fixture(); f8b.readings.seamRegistryText = "";
  r = reconcile(f8b.register, f8b.readings, f8b.plan);
  check(has(r, /接缝「a\.one」登记「已建」/), "负控：已建接缝缺字面量必须红");
  const f8c = fixture(); f8c.readings.remoteSeams = ["x.extra"]; // 等长替换：数量档绿、封闭集合档必须红
  r = reconcile(f8c.register, f8c.readings, f8c.plan);
  check(has(r, /封闭集合破裂/), "负控：远程清单多一条（等长替换）必须红");
  const f8d = fixture(); f8d.readings.remoteSeams = ["a.one", "x.extra"];
  r = reconcile(f8d.register, f8d.readings, f8d.plan);
  check(has(r, /remoteSeamCount.*登记 1 条 \/ 实测 2 条/), "负控：远程清单数量不符必须红并报差值");

  // 负控⑨：实现者计数
  const f9 = fixture(); f9.readings.implementors.TraitOne = 3;
  r = reconcile(f9.register, f9.readings, f9.plan);
  check(has(r, /实现者「TraitOne」：登记 2 \/ 实测 3 \/ 差 1/), "负控：实现者数差 1 必须红并报「登记/实测/差」");

  // 负控⑩：planNumber 缺失 —— hard 锚点红；manual/hard:false 锚点只报告不红
  const f10 = fixture(); anchor(f10, "commandTotal").planNumber = "4242";
  r = reconcile(f10.register, f10.readings, f10.plan);
  check(has(r, /planNumber「4242」/), "负控：hard 锚点 planNumber 缺失必须红（本例同时踩中⑪「自相矛盾」，两条都是真红）");
  const f10b = fixture(); anchor(f10b, "k1Ext").planNumber = "99"; anchor(f10b, "k1Ext").planMustAppear = true;
  r = reconcile(f10b.register, f10b.readings, f10b.plan);
  check(r.hardFails.length === 0 && r.reports.some((x) => x.includes("planNumber「99」未在 PLAN 正文以数字边界形态命中")), "正控⑩b：manual 锚点 planNumber 缺失只报告（planNumber 与 value 保持同数，不触发⑪）");

  // 负控⑪（第 1 条修复）：锚点内部自洽 —— 只改 value 迁就盘上、planNumber 留旧数必须被抓出
  const f11 = fixture(); anchor(f11, "commandTotal").value = 11; f11.readings.registry.commandTotal = 11;
  r = reconcile(f11.register, f11.readings, f11.plan.replace("能力面 10 条 / 3 模块", "能力面 11 条 / 3 模块"));
  check(has(r, /锚点 commandTotal 自相矛盾：value=11 \/ planNumber=10/), "负控⑪a：value 单方面迁就盘上（planNumber 留旧数）必须硬失败并点名锚点");
  // 负控⑫：钉句里留旧数（value/planNumber 同批改了，PLAN 原文没改）
  const f12 = fixture(); anchor(f12, "commandTotal").planPinned = [{ line: 1, mustContain: ["5555", "3"] }];
  r = reconcile(f12.register, f12.readings, f12.plan);
  check(has(r, /锚点 commandTotal 自相矛盾：.*mustContain=5555/), "负控⑫b：planPinned 纯数字 token 与表内登记数脱节必须硬失败");
  // 正控：跨锚点宽容不得放松「兄弟读数」这一档 —— 用直调把「撞别锚点的数」与「谁都不撞」分开验
  const consReg = (anchors) => ({ caliber: { groupCounts: { A: 1, B: 0, C: 0, H: 0, K: 0 }, totalRows: 1, capacityRegister: 1 }, anchors, entries: [] });
  check(checkAnchorConsistency(consReg([
    { id: "x", value: 84873, planNumber: "84 873", planPinned: [{ line: 1, mustContain: ["84 873", "293"] }], value2: 293 },
    { id: "y", value: 3, planNumber: "3" },
  ])).length === 0, "正控⑪c：千位空格 planNumber + 兄弟锚点读数（293=value2）不得误判为矛盾");
  check(checkAnchorConsistency(consReg([{ id: "x", value: 10, planNumber: "10", planPinned: [{ line: 1, mustContain: ["10", "28 文件", "A 11 + B 10"] }] }])).length === 0, "正控⑪d：非纯数字 token（「28 文件」「A 11 + …」）不做数字比对");
  check(checkAnchorConsistency(consReg([{ id: "x", value: 10, planNumber: "12" }])).length === 1, "负控⑪e：planNumber≠value 直调必计 1 条矛盾");
  check(checkAnchorConsistency(consReg([{ id: "x", value: 10, planPinned: [{ line: 1, mustContain: ["77"] }] }])).length === 1, "负控⑪f：钉句数字谁都不撞 ⇒ 矛盾");

  // 负控⑮（2026-11-11 阻断 1）：跨锚点宽容的来源必须只有「hard:true 且机器复检」的锚点。
  // 攻击构造（纸面复演 .tmp/cr-review-sim.mjs 的攻击 1）：hard 锚点把 value 与 planNumber **同改**成
  // 盘上实测 2004 ⇒ ①档自洽、抓不到；钉句里的旧数 2005 靠另一条锚点「藏」进表 ⇒ 旧实现收 soft/manual
  // 锚点的数当宽容，于是②档整条形同虚设、返回空。合成表逐条断言「藏数进软锚点必红」。
  const launder = (donor) => checkAnchorConsistency(consReg([
    { id: "hard", hard: true, check: "k1RsFileCount", value: 2004, planNumber: "2004", planPinned: [{ line: 1, mustContain: ["2004", "2005"] }] },
    donor,
  ]));
  check(launder({ id: "soft", hard: false, check: "registryFileLineCount", value: 2005, planNumber: "2005" }).length === 1, "负控⑮a：把旧数 2005 藏进 hard:false 锚点 ⇒ 钉句 2005 必须仍判红（软锚点不得作跨锚点宽容来源，否则改软锚点即可洗白钉句）");
  check(launder({ id: "manual", hard: false, check: "manual", value: 2005, planNumber: "2005" }).length === 1, "负控⑮b：把旧数藏进 check=manual 锚点（永不判红那类）⇒ 同样必须判红");
  check(launder({ id: "hardButManual", hard: true, check: "manual", value: 2005, planNumber: "2005" }).length === 1, "负控⑮c：hard:true 但 check=manual（未被机器复检）⇒ 其数字仍不得作宽容来源");
  check(launder({ id: "undeclared", check: "k1PubTraitCount", value: 2005, planNumber: "2005" }).length === 1, "负控⑮d：donor 未声明 hard:true ⇒ fail-closed，不给宽容资格");
  check(launder({ id: "donor", hard: true, check: "k1PubTraitCount", value: 2005 }).length === 0, "正控⑮e：合法跨锚点引用（donor 为 hard:true 且机器复检的锚点，value 即被钉的数）不得误判红");
  check(checkAnchorConsistency(consReg([
    { id: "a", hard: true, check: "registryCommandCount", value: 1541, planNumber: "1541", planPinned: [{ line: 1, mustContain: ["1541", "164"] }] },
    { id: "b", hard: true, check: "registryModuleCount", value: 164, planNumber: "164" },
    { id: "c", hard: false, check: "registryFileLineCount", value: 1608 },
  ])).length === 0, "正控⑮f：现盘两条合法宽容（164←commandModules、316←k1PubTraitCount 同型）在收窄后仍不红；hard:false 的 1608 在场也不参与");

  // 负控⑬（第 4 条修复）：planNumber 的数字边界 —— 被更长数字包住 / 日期串里的两位数不再算命中
  const f13 = fixture();
  r = reconcile(f13.register, f13.readings, "登记 110 条 / 30 模块（都被更长数字包住，独立成词的那个数一个也没有）。\n\n### 0.12.4 漂移列举\n旧读数 9 只在本段出现。\n\n### 0.13 下一节\n无。\n");
  check(has(r, /planNumber「10」未在 PLAN 正文以数字边界形态命中/), "负控⑬a：`10` 只在 `110` 这类更长数字里出现 ⇒ 裸子串假绿必须消失");
  check(planNumberHits("发布 2026-11-26，另有 1.26 与 PLAN:26 与 1534/26。", "26").length === 0, "负控⑬b：日期/小数点/行号引用/斜杠配对里的 26 一律不算命中");
  check(planNumberHits("股票 84 873 行前端出包。", "84 873").length === 1, "正控⑬c：千位空格写法仍算命中（不许误伤现盘 15 个锚点）");
  check(planNumberHits("终局能力全集登记 26 条。", "26").length === 1, "正控⑬d：独立成词的 26 算命中");

  // 负控⑭（第 2 条修复）：逐类 anchor check 的「登记≠实测 必被抓出」+ 正控「登记=实测 不假阳性」。
  // 全部走合成读数（fixture），零盘扫描；覆盖度按**镜像表实际用到的 check 种类**核算并印出。
  const covered = new Set();
  for (const c of ANCHOR_CHECK_NEGATIVE_CASES) {
    const bad = fixture(); c.mutate(bad);
    const rb = reconcile(bad.register, bad.readings, bad.plan);
    const inHard = rb.hardFails.some((x) => x.includes(`锚点 ${c.id}`) || x.includes(` ${c.id}：`));
    const inReport = rb.reports.some((x) => x.includes(`锚点 ${c.id}`));
    check(c.tier === "hard" ? inHard : inReport, `负控⑭：check「${c.check}」登记≠实测（锚点 ${c.id}，应落 ${c.tier} 档）必须被抓出并点名，实得 hard=[${rb.hardFails.join(" | ")}] report=[${rb.reports.join(" | ")}]`);
    if (c.tier === "report") { check(!inHard, `负控⑭：check「${c.check}」的 hard:false 锚点 ${c.id} 差异只许「只报告」，不得升成硬失败`); }
    covered.add(c.check);
    // 正控：同一 check 在干净读数下必须存在且不假阳性（hard:false 档干净时同样 PASS，差异时才降为 REPORT）
    if (c.check === "manual") {
      check(rb.reports.some((x) => x.includes("本门机械复算 99")), "负控⑭m：manual 锚点的机械复算读数必须印进报告（登记 99 与实测 99 可对照，且不进 hardFails）");
    } else {
      const cleanRow = cleanResults.find((a) => a.txt.startsWith(`${c.id}（`));
      check(cleanRow && cleanRow.status === "PASS", `正控⑭：check「${c.check}」在干净读数下锚点 ${c.id} 必须 PASS（实得 ${cleanRow ? cleanRow.status : "fixture 无此锚点，覆盖清单与 fixture 不同步"}）`);
    }
  }
  // 覆盖度的分母 = **镜像表实际用到的 check 种类**（只读表本体一次，不扫源码）；表缺失时退回脚本声明清单
  let tableChecks = null;
  let coverageNote = "（按脚本声明清单计）";
  if (existsSync(REGISTER_PATH)) {
    try { tableChecks = new Set(JSON.parse(readFileSync(REGISTER_PATH, "utf8")).anchors.map((a) => a.check)); coverageNote = "（按镜像表实际使用计）"; }
    catch { coverageNote = "（镜像表 JSON 解析失败，退回脚本声明清单）"; }
  } else { coverageNote = "（镜像表缺失，退回脚本声明清单）"; }
  const usedChecks = tableChecks ?? new Set(ANCHOR_CHECK_KINDS_DECLARED);
  for (const k of usedChecks) { check(covered.has(k), `自检覆盖空洞：check 种类「${k}」没有负控+正控 ⇒ 必须补进 ANCHOR_CHECK_NEGATIVE_CASES 与 fixture`); }
  for (const k of covered) { check(ANCHOR_CHECK_KINDS_DECLARED.includes(k), `覆盖清单里的 check「${k}」不在脚本声明清单（头注 data-format 行）内`); }
  if (tableChecks) {
    for (const k of tableChecks) { check(ANCHOR_CHECK_KINDS_DECLARED.includes(k), `镜像表用了声明清单之外的 check 种类「${k}」⇒ 头注 data-format 行与本门 switch 必须同批改（禁止只改表）`); }
  }
  const missing = [...usedChecks].filter((k) => !covered.has(k));
  console.log(`SELFTEST 覆盖度：已覆盖 check 种类 ${covered.size} / 表内实际使用 ${usedChecks.size}${coverageNote}${missing.length ? ` —— 缺 [${missing.join(", ")}]` : " —— 无空洞"}`);

  // ㉑系列（任务 #48，--repin-plan）：定位键唯一才回写 · 歧义 / 零命中 / 缺键一律拒绝写盘 ·
  //   回写后 PLAN 对账必归零 · 只改行号不动判据 · 行号锚点**不得**被全文模糊匹配替代（主代理红线）。
  {
    // 正控㉑d：PLAN 未变 ⇒ 无需重锚（幂等，绝不无谓写盘）
    const f = fixture();
    const rp = computeRepin(f.register, f.plan);
    check(rp.changes.length === 0 && rp.blockers.length === 0 && rp.noKey.length === 0, `正控㉑d：干净读数不得产生任何回写（实得 changes=${rp.changes.length} blockers=[${rp.blockers.join(" | ")}] noKey=[${rp.noKey.join(" | ")}]`);
    check(rp.total === 6 && rp.kept === 6, `正控㉑d2：受行号约束引用应 6 条全相符（1 钉句 + 3 planLine + 2 区间端点；实得 total=${rp.total} kept=${rp.kept}）`);
  }
  {
    // 负控㉑a：删 PLAN 一行（实测过的真实形态）⇒ 其后的引用各 -1，钉句在第 1 行不受影响
    const f = fixture();
    const arr = f.plan.split("\n");
    arr.splice(1, 1);
    const shifted = arr.join("\n");
    const rp = computeRepin(f.register, shifted);
    check(rp.blockers.length === 0 && rp.noKey.length === 0, `负控㉑a：删一行后每条引用都应唯一定位（实得 blockers=[${rp.blockers.join(" | ")}]）`);
    check(rp.changes.length === 5 && rp.kept === 1, `负控㉑a2：应恰好 5 条待回写 + 1 条不动（实得 changes=${rp.changes.length} kept=${rp.kept}）`);
    const to = Object.fromEntries(rp.changes.map((c) => [c.path, c.to]));
    check(to["entries[a.one].planLine"] === 4 && to["entries[k.three].planLine"] === 6, `负控㉑a3：planLine 必须按定位键逐条重算（实得 ${JSON.stringify(to)}）`);
    check(to["entries[b.two].planLineRange[0]"] === 4 && to["entries[b.two].planLineRange[1]"] === 5, "负控㉑a4：planLineRange 两端点各自按定位键回写（不许整段 -1 猜）");
    // 正控㉑e：回写后 PLAN 对账归零 —— 这就是验收条「跑完 --repin-plan，普通运行不再报 PLAN 红」的等价断言
    const next = applyRepinToRegister(f.register, rp.changes);
    const after = checkPlan(next, shifted);
    check(after.fails.length === 0 && after.soft.length === 0, `正控㉑e：回写后 checkPlan 必须零差异（实得 hard=[${after.fails.join(" | ")}] soft=[${after.soft.join(" | ")}]）`);
    // 正控㉑h：重锚**只改行号** —— token / 读数 / 分组等判据字段必须逐字未动
    check(JSON.stringify(stripRepinEditableFields(next)) === JSON.stringify(stripRepinEditableFields(f.register)), "正控㉑h：除行号外的任何字段被改动即违反本模式的前提（判据不许顺手放宽）");
    // 负控㉑i：回写后的表若再被指错行，判定照样红（与 ㉑g 同族，防「重锚把锚点洗成摆设」）
    const mis = JSON.parse(JSON.stringify(next));
    mis.anchors[0].planPinned[0].line = mis.anchors[0].planPinned[0].line + 1;
    const misRes = checkPlan(mis, shifted);
    check(misRes.fails.some((x) => /钉句 PLAN:\d+ 不符/.test(x)), "负控㉑i：行号再指错一行必须照样红（重锚不改变判据形态）");
  }
  {
    // 负控㉑b：定位键**非唯一命中** ⇒ 阻断该项、绝不进 changes（不许从多命中里挑一个）
    const f = fixture();
    const dup = `${f.plan}\n另一处也写 10 条 / 3 模块。\n`; // 第 11 行与第 1 行同命中同一 token 组
    const g = fixture();
    g.register.anchors[0].planPinned[0].line = 4; // 表里的行号漂到别处 ⇒ 需要重锚
    const rp = computeRepin(g.register, dup);
    check(rp.blockers.some((b) => /anchors\[commandTotal\]\.planPinned\[0\]\.line = 4.*非唯一命中.*\[1, 12\]/.test(b)), `负控㉑b：钉句两处命中必须报阻断并列出候选行（实得 blockers=[${rp.blockers.join(" | ")}]）`);
    check(!rp.changes.some((c) => c.path === "anchors[commandTotal].planPinned[0].line"), "负控㉑b2：歧义项不得进 changes");
  }
  {
    // 负控㉑c：**零命中**（句子被改写）⇒ 同样阻断，报「不许猜行、不许插值」
    const f = fixture();
    const g = fixture();
    g.register.anchors[0].planPinned[0].mustContain = ["这句盘上不存在的话"];
    const rp = computeRepin(g.register, f.plan);
    check(rp.blockers.some((b) => /零命中/.test(b)), `负控㉑c：改写后的钉句必须报零命中并阻断（实得 blockers=[${rp.blockers.join(" | ")}]）`);
    const h = fixture();
    h.register.entries[2].planAnchorText = "整行已被改写";
    const rp2 = computeRepin(h.register, h.plan);
    check(rp2.blockers.some((b) => /entries\[k\.three\]\.planLine = 7.*零命中/.test(b)), `负控㉑c2：整行文本键失配必须阻断（实得 blockers=[${rp2.blockers.join(" | ")}]）`);
  }
  {
    // 负控㉑f：**缺定位键** ⇒ 阻断（新增行号引用必须同批带键，否则重锚能力随表增长静默腐烂）
    const f = fixture();
    const g = JSON.parse(JSON.stringify(f.register));
    delete g.entries[1].planAnchorText;
    g.entries[0].planPinned = [{ line: 9, mustContain: [] }];
    g.anchors.push({ id: "noTok", label: "无 token 钉句", value: 1, check: "manual", hard: false, planPinned: [{ line: 3, mustContain: [] }] });
    const rp = computeRepin(g, f.plan);
    check(rp.noKey.some((x) => /entries\[b\.two\]\.planLine/.test(x)), `负控㉑f：登记项缺 planAnchorText 必须记「缺定位键」（实得 noKey=[${rp.noKey.join(" | ")}]）`);
    check(rp.noKey.some((x) => /anchors\[noTok\]\.planPinned\[0\]/.test(x)), "负控㉑f2：钉句无 mustContain token ⇒ 同样缺键（b.two 的 planLineRange[0] 也因缺 planAnchorTextRange 落同一档）");
    check(!rp.changes.some((c) => c.path.startsWith("entries[b.two]")), "负控㉑f3：缺键项绝不进 changes");
  }
  {
    // 负控㉑g（**红线**）：判据仍是「第 N 行含该句」，不是「该句在 PLAN 某处出现」——
    //   把锚点指到同一文档的另一行、句子仍在别处存在时，checkPlan 必须照样红。
    //   这条锁的是「不许改成用模糊匹配代替行号锚点」：一旦有人把 checkPlan 换成全文 includes，
    //   本断言立刻红（真漂移就放掉了）。
    const g = fixture();
    g.register.anchors[0].planPinned[0].line = 6; // 「10 条 / 3 模块」实际在第 1 行
    const rp = checkPlan(g.register, g.plan);
    check(rp.fails.some((x) => /钉句 PLAN:6 不符/.test(x)), "负控㉑g：行号错而句子仍在文档别处 ⇒ 必须判红（行号锚点没被降成全文模糊匹配）");
    const rpRepin = computeRepin(g.register, g.plan);
    check(rpRepin.changes.some((c) => c.path === "anchors[commandTotal].planPinned[0].line" && c.to === 1), "正控㉑g2：同一形态下 --repin-plan 能按 token 唯一定位回第 1 行（重锚治的是行号漂，不是判据）");
  }
  {
    // 负控㉑j：文本版定点回写（真正落盘的那条路径）——needle 唯一才改，不唯一就整次拒绝
    const entry = (seamId, anchorText, planLine, extra = "") => `    { "seamId": "${seamId}", "group": "A", "planAnchorText": ${JSON.stringify(anchorText)}, "planLine": ${planLine}${extra} },`;
    const pinObj = (line) => `    { "id": "commandTotal", "planPinned": [ { "line": ${line}, "mustContain": ["10", "3"] } ] }`;
    const mkTable = (pinLine, rows) => `{\n  "anchors": [\n${pinObj(pinLine)}\n  ],\n  "entries": [\n${rows.join("\n")}\n  ]\n}\n`;
    const arr0 = fixture().plan.split("\n");
    arr0.splice(1, 1);
    const shifted = arr0.join("\n");
    const inline = mkTable(1, [
      entry("a.one", "登记项一 a.one 已建。", 5),
      entry("b.two", "登记项二 b.two 待建。", 6, `, "planLineRange": [5, 6], "planAnchorTextRange": ${JSON.stringify(["登记项一 a.one 已建。", "登记项二 b.two 待建。"])}`),
      entry("k.three", "登记项三 k.three 人工。", 7).replace(/,$/, ""),
    ]);
    const regInline = JSON.parse(inline);
    const rp = computeRepin(regInline, shifted);
    check(rp.blockers.length === 0 && rp.noKey.length === 0, `正控㉑j0：内联表应无阻断（实得 blockers=[${rp.blockers.join(" | ")}] noKey=[${rp.noKey.join(" | ")}]）`);
    const patched = planRepinPatches(inline, regInline, rp.changes);
    check(patched.ok, `正控㉑j：内联排版的表必须能定点替换（实得 problems=[${patched.problems.join(" | ")}]）`);
    const after = patched.text ? JSON.parse(patched.text) : { entries: [] };
    const aOne = after.entries?.find((e) => e.seamId === "a.one");
    const bTwo = after.entries?.find((e) => e.seamId === "b.two");
    check(aOne?.planLine === 4 && bTwo?.planLine === 5 && (bTwo?.planLineRange ?? []).join(",") === "4,5", `正控㉑j2：定点替换后的行号必须与内存版一致（含区间两端点；实得 ${JSON.stringify((after.entries ?? []).map((e) => [e.planLine, e.planLineRange]))}）`);
    check((patched.text ?? "").includes('"mustContain": ["10", "3"]') && (patched.text ?? "").includes('"group": "A"'), "正控㉑j3：判据与其它字段逐字未动（只动行号）");
    // 负控㉑j4：兜底 needle `"planLine": 5` 在表里出现两次（另一条登记项同行号、且它的整行键被拆行排版
    //   ⇒ 带 planAnchorText 的精确 needle 也失效）⇒ **整次拒绝写盘**，不许只改能定位的那一半。
    const twinRow = (seamId, anchorText) => `    {\n    "seamId": "${seamId}",\n    "group": "A",\n    "planAnchorText": ${JSON.stringify(anchorText)},\n    "planLine": 5\n    }`;
    const twin = mkTable(1, [`${twinRow("a.one", "登记项一 a.one 已建。")},`, twinRow("k.three", "登记项三 k.three 人工。")]);
    const regTwin = JSON.parse(twin);
    const rpTwin = computeRepin(regTwin, shifted);
    check(rpTwin.changes.length > 0, `负控㉑j4 前提：夹具必须真的产生待回写项（实得 ${rpTwin.changes.length}）`);
    const bad = planRepinPatches(twin, regTwin, rpTwin.changes);
    check(!bad.ok && bad.problems.some((p) => /找不到唯一的替换锚/.test(p)), `负控㉑j4：替换锚不唯一时必须拒绝定点写盘并给原因（实得 ok=${bad.ok} problems=[${bad.problems.join(" | ")}]）`);
    check(bad.text === null, "负控㉑j5：拒绝写盘时不得返回任何可落盘文本");
  }

  if (fail > 0) { return false; }
  console.log("SELFTEST OK：命令数改红 · 分组漂移红 · 证据行号漂移红 · 未归属非空红 · 旧读数出段红 · 禁语红 · manual 不算通过 · 接缝/远程/实现者负控 · 逐类 anchor check 负控+正控 · 锚点内部自洽（value/planNumber/mustContain）· 藏数进 hard:false/manual 锚点必红（跨锚点宽容只认 hard:true 且机器复检的锚点数）· planNumber 数字边界 · --repin-plan 重锚（唯一命中才回写 / 歧义与零命中与缺键一律拒写 / 回写后 PLAN 对账归零 / 只改行号不动判据 / 行号锚点没被降成全文模糊匹配 / 定点替换不唯一就整次拒绝）· 干净读数零假阳性 全部通过");
  return true;
}

// ── 正式跑：读数复算（一次遍历 + 内容缓存）───────────────────────────────
const loadRegister = () => {
  if (!existsSync(REGISTER_PATH)) { return null; }
  try { return JSON.parse(readFileSync(REGISTER_PATH, "utf8")); } catch { return "BAD"; }
};

/** 扫描面枚举的**唯一实现**（makeWalker 与抖动复点共用，禁另写第二套口径 —— 否则「数到的文件数」
 *  与「判定用的文件清单」不是同一件事，抖动检测本身就失去意义）。 */
function enumerateScanSurface(onFile) {
  (function walk(d) {
    let es;
    try { es = readdirSync(d, { withFileTypes: true }); } catch { return; }
    for (const e of es) {
      if (WALK_PRUNE.has(e.name)) { continue; }
      const p = join(d, e.name);
      if (e.isDirectory()) { walk(p); }
      else if (/\.(rs|ts|tsx|toml)$/.test(e.name)) { onFile(p.slice(ROOT.length + 1).split(sep).join("/")); }
    }
  })(ROOT);
}

function makeWalker() {
  const cache = new Map();
  const contentFor = (rel) => {
    if (!cache.has(rel)) {
      try { cache.set(rel, readFileSync(join(ROOT, rel), "utf8")); } catch { cache.set(rel, null); }
    }
    return cache.get(rel);
  };
  const files = [];
  enumerateScanSurface((rel) => files.push(rel));
  return { files, contentFor };
}

/** 抖动复点：判定跑完后重新统计一次扫描面文件清单数（只走目录树、不读内容 ⇒ 远廉于一次复算）。 */
function countScanSurfaceFiles() {
  let n = 0;
  enumerateScanSurface(() => { n++; });
  return n;
}

function collectReadings(register) {
  const { files, contentFor } = makeWalker();
  const rd = { files, contentFor, walkFiles: files.length };

  // 注册表（lib/command-domains 强制复用，禁止另写正则）
  const regText = contentFor(REG_PATH);
  if (regText !== null) {
    const entries = parseRegistry(regText);
    rd.registry = { commandTotal: entries.length, commandModules: new Set(entries.map((e) => e.mod)).size, unassigned: unassigned(entries), registryFileLines: regText.split("\n").length };
    const modNames = entries.map((e) => e.mod);
    rd.boundary = {};
    for (const [dk, d] of Object.entries(DOMAINS)) {
      const c = {};
      modNames.forEach((m) => { if (d.cmdRe.test(m)) { c[m] = (c[m] || 0) + 1; } });
      rd.boundary[dk] = { total: Object.values(c).reduce((a, b) => a + b, 0), modules: Object.keys(c).length };
    }
    rd.auditDomain = {};
    for (const m of modNames) {
      const dn = domainOf(m);
      rd.auditDomain[dn] ??= { total: 0, modules: new Set() };
      rd.auditDomain[dn].total++;
      rd.auditDomain[dn].modules.add(m);
    }
    for (const v of Object.values(rd.auditDomain)) { v.modules = v.modules.size; }
  } else {
    rd.registry = null; rd.boundary = null; rd.auditDomain = null;
  }

  const auditIn = (rel) => !rel.split("/").some((s) => AUDIT_SKIP.has(s));
  const auditFiles = files.filter(auditIn);

  // 前端体量（脚本文件名口径 + 目录口径）
  {
    let f = 0, ln = 0;
    for (const x of auditFiles) {
      if (/^src\/.*\.(ts|tsx)$/.test(x) && DOMAINS.STOCK.feRe.test(x)) { f++; ln += (contentFor(x) ?? "").split("\n").length; }
    }
    rd.frontend = { files: f, lines: ln };
    rd.frontendDir = {};
    for (const a of register.anchors) {
      if (a.check !== "frontendDirVolume") { continue; }
      let df = 0, dl = 0;
      for (const x of auditFiles) {
        if (x.startsWith(a.dir + "/") && /\.(ts|tsx)$/.test(x)) { df++; dl += (contentFor(x) ?? "").split("\n").length; }
      }
      rd.frontendDir[a.dir] = { files: df, lines: dl };
    }
  }

  // L1 反向阻塞（audit-l1-blockers.mjs:55-77 原口径：foreign = 无任何域 owner 的文件）
  {
    const ownedByAny = (x) => Object.entries(DOMAINS).find(([, d]) => d.ownedRe.test(x))?.[0] || null;
    rd.l1 = {};
    for (const [dk, d] of Object.entries(DOMAINS)) {
      const hitFiles = new Set(); let hits = 0;
      for (const x of auditFiles) {
        if (!x.endsWith(".rs") || ownedByAny(x)) { continue; }
        if (d.crates.some((c) => x.includes("crates/" + c.split("axagent-")[1] + "/"))) { continue; }
        const src = contentFor(x) ?? "";
        for (const l of src.split("\n")) {
          const t = l.trim();
          if (t.startsWith("//") || t.startsWith("///")) { continue; }
          if (d.useRe.test(l)) { hits++; hitFiles.add(x); }
        }
      }
      rd.l1[dk] = { hits, files: hitFiles.size };
    }
  }

  // K1 扫描面：8 根全部 .rs（忽略 target/node_modules/gen/tests）；实现者站点 + pub trait 一次遍历
  const k1Files = files.filter((p) => p.endsWith(".rs") && K1_ROOTS.includes(p.split("/")[0]) && !p.split("/").some((s) => K1_IGNORE.has(s)));
  const pubTraits = new Set();
  const externalTraits = new Set();
  const implementors = {};
  for (const x of k1Files) {
    const s = contentFor(x) ?? "";
    const harness = x.startsWith(HARNESS_PREFIX);
    const dbl = isTestDoublePath(x);
    for (const l of s.split("\n")) {
      const pt = /^pub trait\s+([A-Za-z_][A-Za-z0-9_]*)/.exec(l);
      if (pt) { pubTraits.add(pt[1]); }
      const t = implTraitOf(l);
      if (t && !harness && !dbl) { externalTraits.add(t); implementors[t] = (implementors[t] || 0) + 1; }
    }
  }
  rd.k1 = { rsFiles: k1Files.length, pubTraits: pubTraits.size, externalImplTraits: externalTraits.size };
  rd.implementors = implementors;

  // 远程接缝清单（worker.rs:110 SUPPORTED_REMOTE_SEAMS）
  const wk = contentFor(register.sources.remoteSeamFile);
  if (wk === null) {
    rd.remoteSeams = null;
  } else {
    const lines = wk.split("\n");
    const st = lines.findIndex((l) => l.includes("SUPPORTED_REMOTE_SEAMS") && l.includes("["));
    const seams = [];
    for (let i = st; i >= 0 && i < lines.length; i++) {
      for (const m of lines[i].matchAll(/"([^"]+)"/g)) { seams.push(m[1]); }
      if (i > st && lines[i].includes("];")) { break; }
    }
    rd.remoteSeams = seams;
  }
  rd.seamRegistryText = contentFor(register.sources.seamRegistryFile);
  return rd;
}

// ── --repin-plan 的正式跑（本门**唯一**会写盘的分支：写盘前逐字节备份，写完从盘上复检，复检不过即回滚）──
function runRepin() {
  const register = loadRegister();
  if (register === null || register === "BAD") {
    console.error(`FAIL: 镜像表 ${REGISTER_PATH} ${register === null ? "缺失" : "JSON 解析失败"} —— 没有表就没有可重锚的东西。`);
    return 1;
  }
  const planRel = register.sources?.plan;
  const planAbs = planRel ? join(ROOT, planRel) : null;
  if (!planAbs || !existsSync(planAbs)) {
    console.error(`FAIL: PLAN 缺失（${planRel ?? "sources.plan 未声明"}）—— 无文本可定位 ⇒ 不重锚、不写盘。`);
    return 1;
  }
  const planText = readFileSync(planAbs, "utf8");
  const srcText = readFileSync(REGISTER_PATH, "utf8");
  const { changes, blockers, noKey, kept, total } = computeRepin(register, planText);
  const planLineCount = planText.split("\n").length;

  console.log("\n=== --repin-plan：按定位键把 PLAN 行号引用重锚到当前文本 ===");
  console.log(`受行号约束的引用 ${total} 条 · 已与 PLAN 现状逐行相符 ${kept} · 待回写 ${changes.length} · 阻断 ${blockers.length} · 缺定位键 ${noKey.length}（PLAN 现 ${planLineCount} 行）`);
  for (const c of changes) {
    const d = c.to - c.line;
    console.log(`  重锚 ${c.path}：${c.line} → ${c.to}（${d >= 0 ? "+" : ""}${d} 行）`);
  }
  for (const b of blockers) { console.log(`  阻断 ${b}`); }
  for (const k of noKey) { console.log(`  缺键 ${k}`); }
  const uncovered = [];
  if (register.caliber?.basisLines?.length) { uncovered.push(`caliber.basisLines = [${register.caliber.basisLines.join(", ")}]`); }
  if (register.pipeline?.lines?.length) { uncovered.push(`pipeline.lines = [${register.pipeline.lines.join(", ")}]`); }
  if (uncovered.length) {
    console.log(`  ⚠ 本模式**不重锚**（无定位键的纯文档指针，行号漂了不会红也不会被改写）：${uncovered.join(" · ")} —— PLAN 行数变动后请人工核对这些数字。`);
  }

  if (blockers.length || noKey.length) {
    console.error(`\n拒绝写盘：${blockers.length + noKey.length} 条引用无法**唯一**定位 —— 不许猜行、不许插值、不许从多命中里挑一个。`);
    console.error("修法：把该项的 mustContain 补到只命中一行（或给登记项补 planAnchorText 整行文本键），再重跑本模式。");
    return 1;
  }
  if (!changes.length) {
    console.log("\nPASS: 无需重锚 —— 全部行号引用与 PLAN 现状逐行相符（未写盘、未备份）。");
    return 0;
  }

  const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\..+$/, "");
  const bakPath = join(ROOT, ".tmp", `capability-register.${stamp}.bak.json`);
  if (!existsSync(dirname(bakPath))) { mkdirSync(dirname(bakPath), { recursive: true }); }
  writeFileSync(bakPath, srcText, "utf8");
  console.log(`\n备份：${bakPath.split(sep).join("/")}（${srcText.length} 字符，逐字节）`);

  const patched = planRepinPatches(srcText, register, changes);
  if (!patched.ok) {
    console.error(`\n拒绝写盘（定点替换把握不足）：\n  - ${patched.problems.join("\n  - ")}`);
    console.error(`备份仍在 ${bakPath}，原表未动。`);
    return 1;
  }
  writeFileSync(REGISTER_PATH, patched.text, "utf8");
  console.log(`已回写 ${REGISTER_PATH.split(sep).join("/")}：${changes.length} 处行号（只改行号，判据一字未动）`);

  // 从盘上重读复检：表自身一致性 + 锚点自洽 + PLAN 对账（hard 与 soft 都必须为空才算「重锚成功」）
  const reread = loadRegister();
  if (reread === null || reread === "BAD") {
    writeFileSync(REGISTER_PATH, srcText, "utf8");
    console.error(`复检失败：写盘后的表无法解析 ⇒ 已从备份回滚。`);
    return 1;
  }
  if (JSON.stringify(stripRepinEditableFields(reread)) !== JSON.stringify(stripRepinEditableFields(register))) {
    writeFileSync(REGISTER_PATH, srcText, "utf8");
    console.error("复检失败：除行号外的任何字段被改动（判据被动过）⇒ 已从备份回滚。");
    return 1;
  }
  const cons = [...checkSelfConsistency(reread), ...checkAnchorConsistency(reread)];
  const plan = checkPlan(reread, planText);
  if (cons.length || plan.fails.length || plan.soft.length) {
    writeFileSync(REGISTER_PATH, srcText, "utf8");
    console.error(`复检失败：回写后 PLAN 对账仍有差异 ⇒ 已从备份回滚。\n  - ${[...cons, ...plan.fails, ...plan.soft].join("\n  - ")}`);
    return 1;
  }
  console.log(`\nPASS: 回写后复检通过 —— 钉句 ${reread.anchors.reduce((n, a) => n + (a.planPinned?.length ?? 0), 0)} 处、登记项 planLine ${reread.entries.filter((e) => e.planLine != null).length} 条全部逐行命中；`);
  console.log("      PLAN 侧判据已归零 ⇒ 再跑 `node scripts/check-capability-register.mjs` 时，planPinned / planLine 一族不应再报红。");
  console.log("      ⚠ 本模式只重锚 PLAN 行号，**不碰 evidence（源码 文件:行）** —— 代码侧漂移是该抓的缺陷，自动改写等于洗白。");
  return 0;
}

function main() {
  if (SELFTEST) { process.exit(runSelftest() ? 0 : 1); }
  const T0 = Date.now();
  if (REPIN) { process.exit(runRepin()); }
  const register = loadRegister();
  if (register === null || register === "BAD") {
    console.error(`FAIL: 镜像表 ${REGISTER_PATH} ${register === null ? "缺失" : "JSON 解析失败"} —— 本门唯一权威就是它，跳过 ≠ 通过。`);
    process.exit(1);
  }
  const planText = existsSync(join(ROOT, register.sources.plan)) ? readFileSync(join(ROOT, register.sources.plan), "utf8") : null;

  // 只读调试档：行为与退出码逐字不变（它不做判定，故也不做抖动复点）
  if (DUMP) {
    const readings = collectReadings(register);
    const { files, contentFor, ...plain } = readings;
    console.log(JSON.stringify({ ...plain, seamRegistryText: plain.seamRegistryText ? `<${plain.seamRegistryText.length} chars>` : null, walkFiles: readings.walkFiles }, null, 1));
    console.log(`（--dump-readings 只读不落盘；耗时 ${((Date.now() - T0) / 1000).toFixed(1)}s）`);
    process.exit(0);
  }

  // 判定 + 遍历抖动守卫（口径与理由见头注「遍历抖动」段）：最多两轮；
  // 首轮头尾不一致 ⇒ 本轮不判、重试一次；重试轮仍不收敛 ⇒ 走本仓既有 skip 语义
  // （印警告 + 计入 skipped 摘要 + **不因抖动判红**，exit 码由其余硬失败决定）。
  let judged = null;
  let skipped = null;
  for (let attempt = 1; attempt <= 2; attempt++) {
    const readings = collectReadings(register);
    const res = reconcile(register, readings, planText);
    const before = readings.walkFiles; // 本趟开始时的扫描面文件清单数（makeWalker 先枚举、后才读内容）
    const after = countScanSurfaceFiles(); // 判定跑完后按同一口径复点一次
    judged = { readings, res, before, after, attempt };
    if (before === after) { break; }
    console.log(`⚠ 遍历期间盘上文件数变化（${before}→${after}），本轮不判 —— 跳过 ≠ 通过${attempt === 1 ? "；重试一次。" : "；重试后仍不收敛，按 skip 语义处理。"}`);
    if (attempt === 2) { skipped = `${before}→${after}`; }
  }
  const { res, before, after } = judged;
  const secs = ((Date.now() - T0) / 1000).toFixed(1);
  const volatility = skipped ? ` / ⚠ 抖动跳过 1 项（${skipped} 未收敛，本轮不判）` : judged.attempt > 1 ? ` / 抖动重试 1 次后收敛（${before}=${after}）` : "";

  console.log("\n=== 能力接缝登记表对账 ===");
  console.log(`登记 ${res.counts.total} 行 / 机器复检 ${res.counts.machine} / 只报告 ${res.counts.report} / 未机器复检 ${res.counts.manual}（耗时 ${secs}s，遍历 ${before} 文件 / 复点 ${after} 文件${volatility}）`);
  if (skipped) {
    console.log("── 抖动跳过 1 项（不算通过，也不因抖动判红）──");
    console.log(`⚠ 本趟判定不可信：遍历期间盘上文件数变化（${skipped}），重试一次后仍不收敛 ⇒ 跳过 ≠ 通过。`);
    console.log("   下方判据按重试轮读数逐条印出；硬失败只在「登记与盘上实测真的不符」时成立，抖动本身不新增红，exit 码由其余硬失败决定。");
  }
  // 逐条列表（第 5 条修复）：锚点 / 登记项 / PLAN / 未机器复检 / 硬失败 **一律全量** ——
  // 旧 cap=30 对 36 行登记项每次吞掉 4–6 条 PASS，被省略的恰是最需要看见的那几行；
  // 只有「只报告」这类噪声摘要仍限量 30 条并显式印剩余数（不灌水，也不假绿）。
  const list = (rows, cap = Number.POSITIVE_INFINITY) => {
    rows.slice(0, cap).forEach((x) => console.log(x));
    if (rows.length > cap) { console.log(`… 其余 ${rows.length - cap} 条省略（此档为噪声摘要，不含判据行）`); }
  };
  list(res.anchorResults.map((a) => `${a.status}: 锚点 ${a.txt}`));
  list(res.entryResults.map((e) => `${e.ok ? "PASS" : "FAIL"}: 登记项 ${e.txt}`));
  list(res.planPassed.map((p) => `PASS: ${p}`));
  if (res.reports.length) { console.log("── 只报告（不决定退出码）──"); list(res.reports, 30); }
  if (res.manualLines.length) {
    console.log(`── 未机器复检 ${res.manualLines.length} 行（不算通过，逐条印出）──`);
    list(res.manualLines);
  }
  if (res.hardFails.length) {
    console.log("");
    list(res.hardFails.map((f) => (f.startsWith("FAIL:") ? f : `FAIL: ${f}`)));
    console.error(`\n共 ${res.hardFails.length} 条硬失败 —— 镜像表与盘上实测不一致：改代码或同批改表+PLAN §0.7，不许放宽判据。`);
    process.exit(1);
  }
  console.log(`\nPASS: hard:true 判据全部一致（登记 ${register.entries.length} = ${Object.entries(register.caliber.groupCounts).map(([g, n]) => `${g}${n}`).join("/")}；能力面 Σ=${register.caliber.capacityRegister}；锚点 ${register.anchors.length} 项）。`);
  process.exit(0);
}

// main guard（阻断 2）：只有「本文件被直接执行」才跑主流程。被 `import` 时纯函数照常可用，
// 零盘扫描、零副作用、绝不 process.exit（旧版顶层即扫描 + exit，任何复算纯函数的 import 都会被
// 劫持跑一次全门，实测 33.3s）。argv[1] 缺省（管道 / -e / 被其它解释器载入）⇒ 不匹配 ⇒ 不跑。
const entryPath = process.argv[1];
if (entryPath && import.meta.url === pathToFileURL(entryPath).href) { main(); }
