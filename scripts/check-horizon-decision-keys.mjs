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
//   TS 侧按**大括号深度**只取 interface 顶层键（`legs: Array<{ factor: … }>` 的内层键不算），
//   扫键前**先剥注释**（等长替换成空格）：否则注释里的散文会把
//   `foo::BAR` 中的 `_BAR::` 读成一个键 —— 实证 2026-10-16：`calibrationReady` 的
//   JSDoc 写了 `reflection_stats::CALIBRATION_MIN_SAMPLES` ⇒ 幽灵键 `stats` ⇒ R2/R4 对散文报红
//   （同 `check-reco-horizon-parity.mjs` 的 `stripFullLineComments` 纪律：判据不得被注释触红）。
//   三处解析都带自证（`--dump`），解析不出就当红，不当「没有违规」。

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const read = (rel) => readFileSync(resolve(ROOT, rel), "utf8");

const TIERS = ["ultra-short", "short", "mid", "long"];
const BRANCH_PATH = (t) => `src-tauri/src/commands/portfolio-mgr-h-${t}.rhai`;
const MAIN_PATH = "src-tauri/src/commands/portfolio-mgr.rhai";
const TS_PATH = "src/types/stock-analysis.ts";
// ── R5 的四份来源（2026-10-16，PLAN-llm-horizon-decision-and-parity.md 拍板 1 甲案 + 拍板 2 嵌列）──
// LLM 逐档决策**没有 Rhai 产端**：它的「产端」是 prompt 契约 + schema 文档，归一在 Rust 解析器，
// 读侧是 TS 类型 ⇒ 同一条「键名对不上不报错、只是那一格永远空」的失效机制照旧成立，
// 只是四个源换了一批，所以扩进本门而不是另起一炉（禁区 12：判据也只留这一处）。
const LLM_SCHEMA_PATH = "src-tauri/config/llm-output-schemas.json";
const LLM_PROMPT_PATH = "src-tauri/agency_experts/stock-analysis/trader.md";
const LLM_CAMEL_AUTH_PATH = "src-tauri/crates/harness/src/holding_period.rs";
const LLM_RUST_PARSE_PATH = "src-tauri/src/commands/stock_workflow/decision.rs";
const LLM_TS_READER_PATH = "src/lib/agentOutput.ts";
const LLM_CORE_PATH = "src-tauri/src/commands/stock_workflow/core.rs";
/** B4：逐档结局 2×2 的 DTO 权威（Rust 结构体在 engine 里，TS 镜像在同一份 `stock-analysis.ts`） */
const REFLECTION_STATS_RS_PATH = "src-tauri/crates/analysis-engine/src/reflection_stats.rs";
/** 代际标记的键名（判「键在不在」，不判值 ⇒ Rust 与 TS 只共享这一个串） */
const LLM_CONTRACT_KEY = "horizonDecisionContract";

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

/** 剥掉 TS 的行注释（双斜杠）与块注释（斜杠星号对），**等长替换成空格**（换行保留）——
 *  长度与花括号深度都不变，扫描索引仍与原文对齐，但注释里的 `foo::BAR` 不会再被当成键。
 *  纪律同 `check-reco-horizon-parity.mjs` 的 `stripFullLineComments`：判据不得被注释触红。
 */
function stripTsComments(src) {
  const out = src.split("");
  const blank = (from, to) => {
    for (let k = from; k < to; k += 1) { if (out[k] !== "\n") out[k] = " "; }
  };
  let i = 0;
  while (i < src.length) {
    if (src.startsWith("//", i)) {
      const j = src.indexOf("\n", i);
      const end = j < 0 ? src.length : j;
      blank(i, end);
      i = end;
    } else if (src.startsWith("/*", i)) {
      const j = src.indexOf("*/", i + 2);
      const end = j < 0 ? src.length : j + 2;
      blank(i, end);
      i = end;
    } else {
      i += 1;
    }
  }
  return out.join("");
}

/** TS interface 的**顶层**键（带是否可选）；按大括号深度过滤嵌套类型定义的键 */
export function tsKeys(src, iface = "HorizonDecision") {
  const clean = stripTsComments(src);
  const start = clean.indexOf(`export interface ${iface} {`);
  if (start < 0) return null;
  const body = clean.slice(start + `export interface ${iface} `.length);
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

/**
 * Rust 结构体的 `pub <field>:` 字段名（R5f 用），**按 serde camelCase 折算后**返回。
 *
 * 为什么在这里也要一道对账：DTO 的 Rust 侧字段名与 TS 侧字段名分居两个仓角，
 * 改一个忘一个**不会报错** —— 前端读到的就是 `undefined`，2×2 的格子会静默显示 0
 * （即「LLM 判错 0 条」，一个看起来完全合理的假数）。
 */
export function rustStructFields(src, structName) {
  const clean = stripRsComments(src);
  const start = clean.indexOf(`pub struct ${structName} {`);
  if (start < 0) return null;
  const body = clean.slice(start);
  const end = body.indexOf("\n}");
  const block = end < 0 ? body : body.slice(0, end);
  const out = [];
  for (const m of block.matchAll(/^\s*pub ([a-z][a-z0-9_]*):/gm)) {
    out.push(m[1].replace(/_([a-z])/g, (_, c) => c.toUpperCase()));
  }
  return out.length > 0 ? out : null;
}

/** 去掉 `//` 行注释与 `///` 文档注释（Rust 侧扫描前的骨架化） */
export function stripRsComments(src) {
  return src
    .split("\n")
    .filter((ln) => {
      const t = ln.trim();
      return !(t.startsWith("//") || t.startsWith("/*") || t.startsWith("*"));
    })
    .join("\n");
}

/** 档位 camel 键的**权威**：现场读 harness `Period::camel_key()` 的 match 臂（不手抄四档表） */
export function camelAuthority(src) {
  const lines = src.split("\n");
  const i = lines.findIndex((l) => l.includes("pub fn camel_key("));
  if (i < 0) return null;
  const out = [];
  for (let k = i + 1; k < lines.length; k += 1) {
    const m = lines[k].match(/Period::[A-Za-z]+\s*=>\s*"([A-Za-z][A-Za-z0-9]*)"/);
    if (m) out.push(m[1]);
    if (/^\s{4}\}\s*$/.test(lines[k])) break;
  }
  return out.length === 4 ? out : null;
}

/** schema 文档里 `trader_output.decisions` 的档位键与每档 required 字段集 */
export function llmSchema(jsonText) {
  let doc = null;
  try { doc = JSON.parse(jsonText); } catch { return null; }
  const dec = doc?.schemas?.trader_output?.properties?.decisions;
  if (!dec?.properties) return null;
  const perTierRequired = {};
  for (const [k, v] of Object.entries(dec.properties)) {
    perTierRequired[k] = Array.isArray(v?.required) ? [...v.required].sort() : null;
  }
  return { tiers: Object.keys(dec.properties), perTierRequired, containerRequired: Array.isArray(dec.required) ? [...dec.required].sort() : null };
}

/**
 * prompt（trader.md）里**每一处** `"decisions": {` 的顶层键集。
 * 逐处收而不是收第一处：契约段与输出示例段各写了一遍，只改一段就是「文档自己跟文档打脸」，
 * 而模型读的是整篇 prompt ⇒ 两处必须同时等于权威。
 */
export function llmPromptTiers(md) {
  const MARK = '"decisions": {';
  const sets = [];
  let from = 0;
  for (;;) {
    const i = md.indexOf(MARK, from);
    if (i < 0) break;
    from = i + MARK.length;
    const open = i + MARK.length - 1;
    let depth = 0; let end = -1;
    for (let k = open; k < md.length; k += 1) {
      if (md[k] === "{") depth += 1;
      else if (md[k] === "}") { depth -= 1; if (depth === 0) { end = k; break; } }
    }
    if (end < 0) return null;
    const inner = md.slice(open + 1, end);
    const keys = [];
    let d = 0;
    for (let k = 0; k < inner.length; k += 1) {
      const ch = inner[k];
      if (ch === "{") d += 1;
      else if (ch === "}") d -= 1;
      else if (d === 0 && ch === '"') {
        const j = inner.indexOf('"', k + 1);
        if (j < 0) return null;
        const name = inner.slice(k + 1, j);
        if (/^[A-Za-z][A-Za-z0-9]*$/.test(name) && /^\s*:/.test(inner.slice(j + 1))) keys.push(name);
        k = j;
      }
    }
    if (keys.length === 0) return null;
    sets.push(keys);
  }
  return sets.length > 0 ? sets : null;
}


/**
 * 产端 `core.rs` 往 decision_json 里嵌 `agreementByHorizon` 时写下的那批键（R5d 用）。
 * 只认该 json! 块**顶层**的 `"k":` —— 块里嵌套的对象一层都没有，但一旦有人加嵌套，
 * 深度计数会让内层键不被收（宁少不误，漏收会在 R5d 里表现为「TS 多声明」而不是静默通过）。
 */
export function coreEmbeddedKeys(src, key = "agreementByHorizon") {
  const at = src.indexOf(`"${key}"`);
  if (at < 0) return null;
  const open = src.indexOf("json!(", at);
  if (open < 0) return null;
  let i = src.indexOf("{", open);
  if (i < 0) return null;
  const start = i;
  let depth = 0;
  let end = -1;
  for (let k = start; k < src.length; k += 1) {
    if (src[k] === "{") depth += 1;
    else if (src[k] === "}") { depth -= 1; if (depth === 0) { end = k; break; } }
  }
  if (end < 0) return null;
  const body = src.slice(start + 1, end);
  const keys = [];
  let d = 0;
  for (let k = 0; k < body.length; k += 1) {
    const ch = body[k];
    if (ch === "{" || ch === "[") d += 1;
    else if (ch === "}" || ch === "]") d -= 1;
    else if (d === 0 && ch === '"') {
      const j = body.indexOf('"', k + 1);
      if (j < 0) return null;
      const name = body.slice(k + 1, j);
      if (/^[A-Za-z][A-Za-z0-9]*$/.test(name) && /^\s*:/.test(body.slice(j + 1))) keys.push(name);
      k = j;
    }
  }
  return keys.length > 0 ? keys : null;
}

/** LLM 侧四份来源 + 代际标记两处的整体采集（任一份解析失败 ⇒ null，由 check 判红） */
function collectLlm() {
  return {
    authority: camelAuthority(read(LLM_CAMEL_AUTH_PATH)),
    schema: llmSchema(read(LLM_SCHEMA_PATH)),
    promptTierSets: llmPromptTiers(read(LLM_PROMPT_PATH)),
    tsRows: tsKeys(read(TS_PATH), "LlmDecisionsByHorizon"),
    tsFields: tsKeys(read(TS_PATH), "LlmHorizonDecision"),
    embedded: coreEmbeddedKeys(read(LLM_CORE_PATH)),
    embeddedDiv: coreEmbeddedKeys(read(LLM_CORE_PATH), "divergenceByHorizon"),
    tsAgreement: tsKeys(read(TS_PATH), "HorizonAgreement"),
    tsDiv: tsKeys(read(TS_PATH), "HorizonDivergence"),
    rustHasStamp: read(LLM_RUST_PARSE_PATH).includes(`"${LLM_CONTRACT_KEY}"`),
    tsHasStamp: read(LLM_TS_READER_PATH).includes(LLM_CONTRACT_KEY),
  };
}

/** 主判据：返回违规字符串数组（空 = 通过） */
export function check({ perTier, assembly, ts, llm, confusion }) {
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

  // ── R5：LLM 逐档决策的档位键 / 字段集 / 代际标记四源一致（2026-10-16）──
  //   权威是 harness 的 `Period::camel_key()`；prompt、schema 文档、TS 类型、Rust 归一四处都必须是它，
  //   因为这一侧**没有 Rhai 产端可对照**，漂了不会报错，只会让某一格永远「未给出」。
  if (!llm) {
    bad.push("R5a 采集面缺 LLM 侧来源（门自己的采集面退化也算违规，不当「无违规」）");
  } else {
    const auth = llm.authority;
    if (!auth) bad.push("R5a 权威解析失败：harness `Period::camel_key()` 的 match 臂没读满 4 条");
    const cmp = (label, got) => {
      if (!got) {
        bad.push(`R5a ${label} 解析失败（结构变了就判红，不当「没有违规」）`);
        return;
      }
      if (auth && [...got].sort().join("/") !== [...auth].sort().join("/")) {
        bad.push(`R5a 档位键漂移：${label} = ${[...got].sort().join("/")}，权威 camel_key() = ${auth.join("/")}`);
      }
    };
    cmp("schema `decisions.properties`", llm.schema?.tiers);
    const sets = llm.promptTierSets;
    if (!sets) bad.push("R5a prompt 里读不到任何 `\"decisions\": {` 的顶层键（契约段形状变了？）");
    else sets.forEach((s, i) => cmp(`prompt 第 ${i + 1} 处`, s));
    cmp("TS `LlmDecisionsByHorizon`", llm.tsRows?.map(([k]) => k));

    // 每档字段集：四档之间必须同集，且等于 TS `LlmHorizonDecision` 的**必填**字段集
    const req = llm.schema?.perTierRequired ?? {};
    const shapes = new Set(Object.values(req).map((v) => (v ? v.join("/") : "<无required>")));
    if (shapes.size > 1) {
      bad.push(`R5b 各档 required 不一致（同一份 LLM 逐档决策不得有的档 7 字段、有的档 6）：${[...shapes].join(" ｜ ")}`);
    }
    const one = Object.values(req).find((v) => Array.isArray(v));
    const tsFields = llm.tsFields?.filter(([, optional]) => !optional).map(([k]) => k);
    if (tsFields === undefined || !one || !tsFields) {
      bad.push("R5b 字段集解析失败（TS `LlmHorizonDecision` 或 schema 每档 required 读不出来）");
    } else if ([...one].sort().join("/") !== [...tsFields].sort().join("/")) {
      bad.push(`R5b 字段集漂移：schema required = ${[...one].sort().join("/")}，TS 必填 = ${[...tsFields].sort().join("/")}`);
    }

    // 代际标记：Rust 盖的串与 TS 判的串必须是同一个字面量，否则「上线前」那一格静默消失
    // R5d 逐档对照：产端 core.rs 嵌出的 JSON 键 ↔ TS `HorizonAgreement` 字段集。
    //   这一格没有 Rhai 产端可对照，键名对不上**不报错**，只会让面板那一列永远 undefined。
    if (!llm.embedded || !llm.tsAgreement) {
      bad.push("R5d 采集失败：core.rs 的 `agreementByHorizon` json! 块或 TS `HorizonAgreement` 读不出来");
    } else {
      const declared2 = new Set(llm.tsAgreement.map(([k]) => k));
      const emitted = new Set(llm.embedded);
      for (const k of emitted) {
        if (!declared2.has(k)) bad.push(`R5d 产端嵌出但 TS 未声明：${k} ⇒ 前端拿不到，等于没产出`);
      }
      for (const k of declared2) {
        if (!emitted.has(k)) bad.push(`R5d TS 声明但产端从不嵌出：${k} ⇒ 永远 undefined，会被读成「没有这一档」`);
      }
    }

    // R5e 逐档归因：产端嵌出键 ↔ TS `HorizonDivergence` 字段集（与 R5d 同一失效族）
    if (!llm.embeddedDiv || !llm.tsDiv) {
      bad.push("R5e 采集失败：core.rs 的 `divergenceByHorizon` json! 块或 TS `HorizonDivergence` 读不出来");
    } else {
      const dDeclared = new Set(llm.tsDiv.map(([k]) => k));
      const dEmitted = new Set(llm.embeddedDiv);
      for (const k of dEmitted) {
        if (!dDeclared.has(k)) bad.push(`R5e 产端嵌出但 TS 未声明：${k} ⇒ 前端拿不到`);
      }
      for (const k of dDeclared) {
        if (!dEmitted.has(k)) bad.push(`R5e TS 声明但产端从不嵌出：${k} ⇒ 永远 undefined`);
      }
    }

    if (!llm.rustHasStamp) bad.push(`R5c Rust 解析器不再盖 \`${LLM_CONTRACT_KEY}\` ⇒ 读侧四态塌回三态，历史行会被读成「四档都没给」`);
    if (!llm.tsHasStamp) bad.push(`R5c TS 读侧不再判 \`${LLM_CONTRACT_KEY}\` ⇒ 「上线前样本」与「该档未给出」不可区分`);

    // R5f 逐档结局 2×2：Rust `HorizonConfusionRow` 字段（按 serde camelCase 折算）
    //   ↔ TS `HorizonConfusionRow` 字段集。与 R5d/R5e 同一失效族 —— 两侧分居两个仓角，
    //   改一个忘一个不报错，只会让 2×2 的格子显示成 0（看起来完全合理的假数）。
    if (!confusion?.rust || !confusion?.ts) {
      bad.push("R5f 采集失败：Rust `HorizonConfusionRow` 或 TS `HorizonConfusionRow` 读不出来");
    } else {
      const rset = new Set(confusion.rust);
      const tset = new Set(confusion.ts);
      for (const k of rset) {
        if (!tset.has(k)) bad.push(`R5f Rust 有而 TS 未声明：${k} ⇒ 前端读不到，格子会是 undefined`);
      }
      for (const k of tset) {
        if (!rset.has(k)) bad.push(`R5f TS 声明而 Rust 从不产出：${k} ⇒ 永远 undefined`);
      }
    }
  }
  return bad;
}

function collect() {
  const perTier = {};
  for (const t of TIERS) perTier[t] = branchKeys(read(BRANCH_PATH(t)));
  return {
    perTier,
    assembly: assemblyKeys(read(MAIN_PATH)),
    ts: tsKeys(read(TS_PATH)),
    llm: collectLlm(),
    confusion: {
      rust: rustStructFields(read(REFLECTION_STATS_RS_PATH), "HorizonConfusionRow"),
      ts: tsKeys(read(TS_PATH), "HorizonConfusionRow")?.map(([k]) => k) ?? null,
    },
  };
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
    // ── R5 三条负控：每条只打红自己那一格（拆错守卫的负控等于另一种假绿）──
    ["R5a schema 少一档", () => {
      const m = structuredClone(base);
      m.llm.schema.tiers = m.llm.schema.tiers.filter((x) => x !== "ultraShort");
      return check(m);
    }],
    ["R5a prompt 两处不一致", () => {
      const m = structuredClone(base);
      m.llm.promptTierSets[m.llm.promptTierSets.length - 1] = ["ultraShort", "short", "mid"];
      return check(m);
    }],
    ["R5b 各档 required 不等", () => {
      const m = structuredClone(base);
      m.llm.schema.perTierRequired.mid = m.llm.schema.perTierRequired.mid.filter((f) => f !== "verdict");
      return check(m);
    }],
    ["R5b TS 字段集漂移", () => {
      const m = structuredClone(base);
      m.llm.tsFields = m.llm.tsFields.filter(([k]) => k !== "verdict");
      return check(m);
    }],
    ["R5d 产端多嵌一键", () => {
      const m = structuredClone(base);
      m.llm.embedded.push("inventedByCoreOnly");
      return check(m);
    }],
    ["R5d TS 多声明一键", () => {
      const m = structuredClone(base);
      m.llm.tsAgreement.push(["phantomAgreementField", true]);
      return check(m);
    }],
    ["R5e TS 多声明一键", () => {
      const m = structuredClone(base);
      m.llm.tsDiv.push(["phantomDriverField", true]);
      return check(m);
    }],
    ["R5c 代际标记读侧失明", () => {
      const m = structuredClone(base);
      m.llm.tsHasStamp = false;
      return check(m);
    }],
    // ── R5f 两条负控：两侧各打一边（只打一边＝另一半的漂移仍能过门）──
    ["R5f Rust 多一字段", () => {
      const m = structuredClone(base);
      m.confusion.rust.push("inventedByRustOnly");
      return check(m);
    }],
    ["R5f TS 多声明一字段", () => {
      const m = structuredClone(base);
      m.confusion.ts.push("phantomConfusionField");
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
    console.log("✓ 正控：现网键集一致（公式侧 R1-R4 + LLM 侧 R5a-R5e + 结局 2×2 的 R5f 全清）");
  }
  console.log(fail === 0 ? "\nselftest 全过" : `\nselftest 失败 ${fail} 项`);
  process.exit(fail === 0 ? 0 : 1);
}

function dump(x) {
  for (const t of TIERS) console.log(`${t} (${x.perTier[t]?.length ?? "解析失败"})`, x.perTier[t]?.join(" "));
  console.log(`assembly (${x.assembly?.length})`, x.assembly?.join(" "));
  console.log(`TS (${x.ts?.length})`, x.ts?.map(([k, o]) => o ? `${k}?` : k).join(" "));
  console.log("R5 权威 camel_key", x.llm?.authority?.join("/"));
  console.log("R5 schema 档位", x.llm?.schema?.tiers?.join("/"), "| 每档字段", Object.values(x.llm?.schema?.perTierRequired ?? {})[0]?.length);
  console.log("R5 prompt 各处", (x.llm?.promptTierSets ?? []).map((s) => s.length).join("/"), "键 | TS 行", x.llm?.tsRows?.map(([k]) => k).join("/"), "| TS 字段", x.llm?.tsFields?.length);
  console.log("R5d 产端嵌出", (x.llm?.embedded ?? []).length, "键 | TS HorizonAgreement", (x.llm?.tsAgreement ?? []).map(([k, o]) => (o ? `${k}?` : k)).join("/"));
  console.log("R5e 产端嵌出", (x.llm?.embeddedDiv ?? []).length, "键 | TS HorizonDivergence", (x.llm?.tsDiv ?? []).map(([k, o]) => (o ? `${k}?` : k)).join("/"));
}

// 只在**直接执行**时跑 CLI。被 `import` 复用解析器时（例如拿 HEAD 源文自证判据有没有电）
// 不得顺手跑门禁、更不得 `process.exit` —— 那会把调用方整段吞掉，看起来像「门跑过了」。
const invokedDirectly = typeof process.argv[1] === "string"
  && pathToFileURL(process.argv[1]).href === import.meta.url;

if (invokedDirectly) {
  if (process.argv.includes("--selftest")) {
    selftest();
  } else if (process.argv.includes("--dump")) {
    dump(collect());
  } else {
    const bad = check(collect());
    if (bad.length === 0) {
      console.log("OK  逐档决策键集一致：公式侧（四档分支 + 装配 + TS HorizonDecision，R1-R4）＋ LLM 侧（prompt / schema / camel_key 权威 / TS 类型 / 代际标记，R5a-R5e）＋ 结局 2×2（Rust↔TS 字段集，R5f）");
      process.exit(0);
    }
    console.log(`FAIL  ${bad.length} 条键集漂移：`);
    for (const b of bad) console.log(`  · ${b}`);
    process.exit(1);
  }
}
