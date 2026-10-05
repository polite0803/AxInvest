// HorizonDecision 键集一致性门（任务 #34，PLAN §五十五 第 3 条 / §五十六 B2-3 的前置）
//
// ── 为什么必须有这道门 ──
// 逐档决策的**生产者**是四份 Rhai 分支脚本，**声明**是 TS 的 `HorizonDecision`，
// **读侧**还有 Rust（`reflection_stats.rs`）与落库 JSON。三者此前**没有任何锁**：
// `stock_workflow/decision.rs:734-738` 自己写着「该清单与四份分支脚本返回段靠人工对齐，
// 仓内没有锁它的门」。已有漂移实证：读侧 `reflection_stats.rs:49-69` 长期留着 TS 早已
// 按幽灵字段删除的 `conf_lower_bound`；而 TS 侧也删过一次「永不到货的幽灵声明」
// `confidenceRiskAdjusted`（见 `src/types/stock-analysis.ts` 的 confidence 字段注）。
//
// ── 症状为什么必须靠门而不是靠人眼 ──
// 键名对不上**不报错**：产端多写的键前端静默丢（那一格永远空），TS 多声明的键读侧
// 永远 undefined（面板按「没有这一档」处理）。本仓已为「snake 产端 × camel 读端 ⇒
// 超短档价位行从不显示」付过一次代价 —— 同一失效机制。
//
// ── 四条判据 ──
//   R1 产端有、TS 未声明          ⇒ 红（前端拿不到，等于没产出）
//   R2 TS 必填、四档+装配都不产   ⇒ 红（幽灵必填字段）
//   R3 装配专属键（绝对价）出现在**分支脚本**里 ⇒ 红（同一档两个价位来源，v125 修过的那族）
//   R4 TS 声明了、任何一档与装配都不产 ⇒ 红（幽灵可选字段）
//
// ── 用法 ──
//   node scripts/check-horizon-decision-keys.mjs            # 门禁模式
//   node scripts/check-horizon-decision-keys.mjs --selftest # 四条负控（每条必须真的能红）
//   node scripts/check-horizon-decision-keys.mjs --dump     # 打三份键集，排漂移时用
// 退出码：0 = 通过；1 = 有违规或负控失效。
//
// ⚠ 解析面说明（本门刻意只做**字面量**解析，不跑 Rhai）：
//   分支脚本的输出块 = 文件里**最后一个**独占一行的 `#{` 到与之配对的行首 `}`；
//   键必须行首缩进后紧跟 `ident:`。装配追加键 = `8< assembly-begin` 段内的 `row["k"] =`。
//   TS 侧按**大括号深度**只取 interface 顶层键（`legs: Array<{ factor: … }>` 的内层键不算）。
//   三处解析都带自证（`--dump`），解析不出就当红，不当「没有违规」。

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const read = (rel) => readFileSync(resolve(ROOT, rel), "utf8");

const TIERS = ["ultra-short", "short", "mid", "long"];
const BRANCH_PATH = (t) => `src-tauri/src/commands/portfolio-mgr-h-${t}.rhai`;
const MAIN_PATH = "src-tauri/src/commands/portfolio-mgr.rhai";
const TS_PATH = "src/types/stock-analysis.ts";

/** 装配段负责追加的键（绝对价只在**真出仓位**时由主链算，见 portfolio-mgr.rhai 装配段注释） */
const ASSEMBLY_OWNED = ["targetPrice", "stopLoss"];

/** 分支脚本输出字面量的键集；解析不到 ⇒ 返回 null（调用方按红处理，不得当空集） */
export function branchKeys(src) {
  const lines = src.split("\n");
  let start = -1;
  for (let i = lines.length - 1; i >= 0; i -= 1) {
    if (/^#\{\s*$/.test(lines[i])) {
      start = i;
      break;
    }
  }
  if (start < 0) return null;
  const keys = [];
  for (let i = start + 1; i < lines.length; i += 1) {
    if (/^\}/.test(lines[i])) {
      return keys.length > 0 ? keys : null;
    }
    const m = lines[i].match(/^\s+([A-Za-z][A-Za-z0-9]*)\s*:/);
    if (m) keys.push(m[1]);
  }
  return null; // 没遇到配对行首 `}` ⇒ 结构不是预期的输出块
}

/** 主链装配段 `row["k"] =` 追加的键集 */
export function assemblyKeys(src) {
  const begin = src.indexOf("8< assembly-begin");
  if (begin < 0) return null;
  // 装配段的绝对价赋值有两处（`if price_ok` 分支与 else 分支），只认**写入**形态
  const keys = new Set();
  const re = /row\["([A-Za-z][A-Za-z0-9]*)"\]\s*=/g;
  const tail = src.slice(begin);
  let m;
  while ((m = re.exec(tail)) !== null) keys.add(m[1]);
  return [...keys];
}

/** TS interface 的**顶层**键（带是否可选）；按大括号深度过滤嵌套类型定义的键 */
export function tsKeys(src, iface = "HorizonDecision") {
  const start = src.indexOf(`export interface ${iface} {`);
  if (start < 0) return null;
  const body = src.slice(start + `export interface ${iface} `.length);
  let depth = 0;
  const out = [];
  for (let i = 0; i < body.length; i += 1) {
    const ch = body[i];
    if (ch === "{") depth += 1;
    else if (ch === "}") {
      depth -= 1;
      if (depth === 0) break;
    }
    if (depth === 1) {
      const m = body.slice(i).match(/^([A-Za-z][A-Za-z0-9]*)(\??):/);
      if (m) {
        out.push([m[1], m[2] === "?"]);
        i += m[0].length - 1;
      }
    }
  }
  return out;
}

/** 主判据：返回违规字符串数组（空 = 通过） */
export function check({ perTier, assembly, ts }) {
  const bad = [];
  const produced = new Map(); // key → 产它的档位列表
  for (const [tier, keys] of Object.entries(perTier)) {
    if (keys === null) {
      bad.push(`分支脚本 ${tier} 的输出字面量解析失败（结构变了还是 include 路径错了？）`);
      continue;
    }
    for (const k of new Set(keys)) {
      if (!produced.has(k)) produced.set(k, []);
      produced.get(k).push(tier);
    }
  }
  if (ts === null) return ["TS `HorizonDecision` 解析失败"];
  if (assembly === null) return ["主链装配段解析失败（`8< assembly-begin` 锚点没了？）"];

  const declared = new Map(ts);
  const producedSet = new Set(produced.keys());
  const assemblySet = new Set(assembly);

  // R1 产端有、TS 未声明
  for (const k of producedSet) {
    if (!declared.has(k)) bad.push(`R1 产端键未声明：${[...produced.get(k)].join("/")} 产出 \`${k}\`，但 TS HorizonDecision 没有它 ⇒ 前端静默丢弃`);
  }
  // R2 TS 必填 ⇒ 必须四档全产，或由装配追加
  for (const [k, optional] of declared) {
    if (optional) continue;
    const allTiers = produced.get(k) && produced.get(k).length === TIERS.length;
    if (!allTiers && !assemblySet.has(k)) {
      bad.push(`R2 幽灵必填字段：TS 声明 \`${k}:\`（必填），但只有 ${produced.get(k)?.join("/") ?? "没有一档"} 产出${assemblySet.has(k) ? "" : "，装配段也不追加"}`);
    }
  }
  // R3 装配专属键不得出现在分支输出里（同一档两个价位来源）
  for (const k of ASSEMBLY_OWNED) {
    if (producedSet.has(k)) bad.push(`R3 来源重复：绝对价 \`${k}\` 由装配段追加，却出现在分支输出里 ⇒ 同屏两数`);
    if (!assemblySet.has(k)) bad.push(`R3 装配段不再追加 \`${k}\`（前端仍按必填读它）`);
  }
  // R4 TS 声明了但无处生产
  for (const [k] of declared) {
    if (!producedSet.has(k) && !assemblySet.has(k)) bad.push(`R4 幽灵声明：TS 有 \`${k}\`，四档与装配都不产`);
  }
  return bad;
}

function collect() {
  const perTier = {};
  for (const t of TIERS) perTier[t] = branchKeys(read(BRANCH_PATH(t)));
  return { perTier, assembly: assemblyKeys(read(MAIN_PATH)), ts: tsKeys(read(TS_PATH)) };
}

/** 四条负控：把判据打在**已知会红**的样本上，证明它不是恒真断言 */
function selftest() {
  const base = collect();
  const cases = [
    ["R1 未声明的产端键", () => {
      const m = structuredClone(base);
      m.perTier.mid.push("inventedByBranchOnly");
      return check(m);
    }],
    ["R2 必填但某档不产", () => {
      const m = structuredClone(base);
      m.perTier.long = m.perTier.long.filter((k) => k !== "action");
      return check(m);
    }],
    ["R3 分支重复产绝对价", () => {
      const m = structuredClone(base);
      m.perTier.short.push("targetPrice");
      return check(m);
    }],
    ["R4 TS 幽灵声明", () => {
      const m = structuredClone(base);
      m.ts.push(["phantomField", true]);
      return check(m);
    }],
  ];
  let fail = 0;
  for (const [name, run] of cases) {
    const bad = run();
    const hit = bad.some((b) => b.startsWith(name.split(" ")[0]));
    if (!hit) {
      console.log(`✖ 负控失效：${name} 没有让判据变红（⇒ 该判据恒真）`);
      fail += 1;
    } else {
      console.log(`✓ 负控生效：${name} → ${bad.find((b) => b.startsWith(name.split(" ")[0])).slice(0, 96)}…`);
    }
  }
  // 正控：未变异的真实三份必须干净
  const clean = check(base);
  if (clean.length > 0) {
    console.log(`✖ 正控失效：未变异的树已有 ${clean.length} 条违规 ⇒ 负控与正控走的是同一条红路，无从区分`);
    for (const c of clean) console.log(`   ${c}`);
    fail += 1;
  } else {
    console.log("✓ 正控：现网三份键集一致（R1-R4 全清）");
  }
  console.log(fail === 0 ? "\nselftest 全过" : `\nselftest 失败 ${fail} 项`);
  process.exit(fail === 0 ? 0 : 1);
}

function dump(x) {
  for (const t of TIERS) console.log(`${t} (${x.perTier[t]?.length ?? "解析失败"})`, x.perTier[t]?.join(" "));
  console.log(`assembly (${x.assembly?.length})`, x.assembly?.join(" "));
  console.log(`TS (${x.ts?.length})`, x.ts?.map(([k, o]) => o ? `${k}?` : k).join(" "));
}

if (process.argv.includes("--selftest")) selftest();
else if (process.argv.includes("--dump")) dump(collect());
else {
  const bad = check(collect());
  if (bad.length === 0) {
    console.log("OK  HorizonDecision 键集与四档分支输出 + 装配追加一致（R1-R4 全清）");
    process.exit(0);
  }
  console.log(`FAIL  ${bad.length} 条键集漂移：`);
  for (const b of bad) console.log(`  · ${b}`);
  process.exit(1);
}
