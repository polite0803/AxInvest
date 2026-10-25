#!/usr/bin/env node
/**
 * data-quality 槽位缩写表的三处对拍门（`analyst_feedbacks` 全库 0 行的根因之一）。
 *
 * ## 为什么需要它
 *
 * 「槽位缩写（mk/sent/fund/…）→ 分析师节点 id」这张表在仓里有**三处载体**，
 * 且**三处都是活的**（任一处漂移都不会报错，只会让那一格静默失效）：
 *
 * | 载体 | 位置 | 谁消费它 |
 * |---|---|---|
 * | **rhai 脚本体** | `src-tauri/src/commands/data-quality.rhai` 的 `extract_conf(<abbr>_verdict)` | 逐节点诊断产出（`diagnostics` / `diagnostics_by_tier` 的键） |
 * | **Rust 登记表** | `src-tauri/crates/harness/src/holding_period.rs` 的 `DQ_ABBR_TO_BASE` | 种子的 40 键 `input_mapping` + `analyst_feedbacks` 写侧的 `node_id` |
 * | **前端镜像表** | `src/lib/dataQualityDiagnosis.ts` 的 `EXPERT_ID_TO_DQ_ABBR` | 弹窗把诊断条目映射回分析师、反馈落库的 `node_id` 键形 |
 *
 * 漂移的症状（本仓已实测过同族形态）：
 * - rhai 产出的诊断键 `mk` 在前端表里查不到 ⇒ 弹窗该行**空白**；
 * - 前端表多一个缩写 ⇒ 反馈写侧给**不存在的槽位**造行；
 * - Rust 表少一个缩写 ⇒ 反馈表那一行**永远不写**（`analyst_feedbacks` 全库 0 行）。
 *
 * 关键点：**三处都必须一致，且一致的是「缩写 ↔ 节点 id」的配对**，
 * 不是「各自数量相等」。数量相等而配对错位，同样是查表恒落空。
 *
 * ## 判据
 *
 * ① **槽位集合一致**：rhai 脚本体用到的 `<abbr>_verdict` 变量名集合 ==
 *    Rust `DQ_ABBR_TO_BASE` 的缩写集合 == TS `EXPERT_ID_TO_DQ_ABBR` 的值集合。
 * ② **配对一致**：Rust 表与 TS 表逐槽位的「缩写 → 节点 id」必须相等；
 *    rhai 头部注释里那张对照表也逐槽位相等（注释不是文档，是第三份登记）。
 * ③ **扫描面非零**：任一侧解析到的槽位数 < 10 ⇒ 判据失效（exit 2）——
 *    「扫到 0 条」的门等于没有门（本仓已多次为这类假绿付过代价）。
 * ④ **写侧键形态闭环**：反馈表的 `node_id` 必须取 `dq_feedback_node_id(base)`
 *    （剥 `a-` 前缀、**不带档后缀**）—— 唯一读侧 `evolve_node` 按 `NodeId.eq(...)`
 *    精确过滤，调用方传的就是去前缀的 base；写侧若加 `a-` 或加 `--mid`，
 *    新行一条也查不到（症状与前端缩写表漂移一模一样）。
 *
 * 用法：
 *   node scripts/check-dq-abbr-parity.mjs            # 比对
 *   node scripts/check-dq-abbr-parity.mjs --selftest # 正负对照（证明它会红）
 */

import fs from "node:fs";
import path from "node:path";
import process from "node:process";

const ROOT = path.resolve(import.meta.dirname, "..");
const RHAI = "src-tauri/src/commands/data-quality.rhai";
const RUST_REG = "src-tauri/crates/harness/src/holding_period.rs";
const TS_REG = "src/lib/dataQualityDiagnosis.ts";
const WRITER = "src-tauri/src/commands/analyst_feedback.rs";
/** 槽位下限：任一侧解析到的槽位数低于它 ⇒ 判据失效（不是「不一致」，是「没扫到」）。 */
const MIN_SLOTS = 10;

function readRel(rel) {
  try { return fs.readFileSync(path.join(ROOT, rel), "utf8"); } catch { return null; }
}

/** 解析 rhai 脚本体里实际用到的槽位变量：`extract_conf(<abbr>_verdict)`。 */
function parseRhaiSlots(src) {
  const out = new Set();
  const re = /extract_conf\(\s*([a-z]{2,5})_verdict\s*\)/g;
  let m;
  while ((m = re.exec(src)) !== null) { out.add(m[1]); }
  return [...out].sort();
}

/** 解析 rhai 头部注释里的对照表：`//   mk   → a-market-analyst (market-analyst)`。 */
function parseRhaiHeader(src) {
  const out = {};
  const re = /^\/\/\s+([a-z]{2,5})\s+→\s+([a-z0-9-]+)/gm;
  let m;
  while ((m = re.exec(src)) !== null) { out[m[1]] = m[2]; }
  return out;
}

/** 解析 Rust 登记表 `pub const DQ_ABBR_TO_BASE: &[(&str, &str)] = &[ … ]`。 */
function parseRust(src) {
  const start = src.indexOf("pub const DQ_ABBR_TO_BASE");
  if (start < 0) { return { ok: false, reason: "未见 DQ_ABBR_TO_BASE（常量改名或搬家？）" }; }
  const end = src.indexOf("];", start);
  if (end < 0) { return { ok: false, reason: "DQ_ABBR_TO_BASE 收尾定位失败" }; }
  const out = {};
  const re = /\(\s*"([a-z]{2,5})"\s*,\s*"([a-z0-9-]+)"\s*\)/g;
  let m;
  while ((m = re.exec(src.slice(start, end))) !== null) { out[m[1]] = m[2]; }
  return { ok: true, map: out };
}

/** 解析 TS 镜像表 `export const EXPERT_ID_TO_DQ_ABBR: Readonly<Record<string, string>> = { … }`。 */
function parseTs(src) {
  const start = src.indexOf("export const EXPERT_ID_TO_DQ_ABBR");
  if (start < 0) { return { ok: false, reason: "未见 EXPERT_ID_TO_DQ_ABBR（常量改名或搬家？）" }; }
  const end = src.indexOf("\n};", start);
  if (end < 0) { return { ok: false, reason: "EXPERT_ID_TO_DQ_ABBR 收尾定位失败" }; }
  const out = {};
  const re = /"([a-z0-9-]+)"\s*:\s*"([a-z]{2,5})"/g;
  let m;
  while ((m = re.exec(src.slice(start, end))) !== null) { out[m[2]] = m[1]; }
  return { ok: true, map: out };
}

/** 判据①②：三处槽位集合与配对逐一对拍。 */
function compareSlots(rhaiSlots, rhaiHeader, rustMap, tsMap) {
  const errs = [];
  const sets = {
    rhai脚本体: new Set(rhaiSlots),
    rhai头部注释: new Set(Object.keys(rhaiHeader)),
    Rust登记表: new Set(Object.keys(rustMap)),
    前端镜像表: new Set(Object.keys(tsMap)),
  };
  const names = Object.keys(sets);
  const union = new Set();
  for (const n of names) { for (const v of sets[n]) { union.add(v); } }
  // ① 集合一致：任一侧独有 ⇒ 红（并点名是谁独有）
  for (const n of names) {
    const missing = [...union].filter((v) => !sets[n].has(v)).sort();
    if (missing.length) {
      errs.push(`① ${n} 缺槽位: ${missing.join(", ")}（该槽位在这张表里查不到 ⇒ 那一格静默失效）`);
    }
  }
  // ② 配对一致：同一缩写在三处的节点 id 必须相同
  for (const abbr of [...union].sort()) {
    const rust = rustMap[abbr];
    const ts = tsMap[abbr];
    if (rust && ts && rust !== ts) {
      errs.push(`② 缩写 '${abbr}' 的节点 id 不一致：Rust=${rust} / 前端=${ts}（查表恒落空）`);
    }
    const head = rhaiHeader[abbr];
    if (head && rust && head !== rust) {
      errs.push(`② 缩写 '${abbr}' 的节点 id 不一致：rhai头部注释=${head} / Rust=${rust}`);
    }
    if (head && ts && head !== ts) {
      errs.push(`② 缩写 '${abbr}' 的节点 id 不一致：rhai头部注释=${head} / 前端=${ts}`);
    }
  }
  return errs;
}

/** 判据③：扫描面自证 —— 解析到 0 条 = 门失效，必须比「值漂移」更早报出来。 */
function surfaceCheck(rhaiSlots, rustMap, tsMap) {
  const errs = [];
  if (rhaiSlots.length < MIN_SLOTS) {
    errs.push(`③ rhai 脚本体只解析到 ${rhaiSlots.length} 个槽位（< ${MIN_SLOTS}）⇒ 解析器失效，结论不可信`);
  }
  if (Object.keys(rustMap).length < MIN_SLOTS) {
    errs.push(`③ Rust 登记表只解析到 ${Object.keys(rustMap).length} 个槽位（< ${MIN_SLOTS}）⇒ 解析器失效，结论不可信`);
  }
  if (Object.keys(tsMap).length < MIN_SLOTS) {
    errs.push(`③ 前端镜像表只解析到 ${Object.keys(tsMap).length} 个槽位（< ${MIN_SLOTS}）⇒ 解析器失效，结论不可真`);
  }
  return errs;
}

/**
 * 判据④：写侧键形态闭环。
 *
 * 唯一读侧 `evolve_node` 按 `NodeId.eq(...)` **精确**过滤，调用方传的是
 * 「去 `a-` 前缀、不带档后缀」的 base ⇒ 写侧必须写同一个形态。
 */
function checkWriteSideKeyShape(read) {
  const errs = [];
  const holding = read(RUST_REG);
  if (holding === null) { return [`④ 读不到 ${RUST_REG} ⇒ 判据失效`]; }
  const fnStart = holding.indexOf("pub fn dq_feedback_node_id");
  if (fnStart < 0) {
    errs.push("④ 未见 dq_feedback_node_id（改名请同步本门）⇒ 无法确认写侧的 node_id 形态");
    return errs;
  }
  const fnEnd = holding.indexOf("\n}", fnStart);
  const body = fnEnd > fnStart ? holding.slice(fnStart, fnEnd) : holding.slice(fnStart, fnStart + 400);
  if (!body.includes('strip_prefix("a-")')) {
    errs.push("④ dq_feedback_node_id 未剥 `a-` 前缀 ⇒ 写侧写的 id 与读侧查询的 base 恒不相等（新行一条也查不到）");
  }
  if (body.includes('"--"') || body.includes("format!")) {
    errs.push("④ dq_feedback_node_id 带档后缀 / 重新拼串 ⇒ 写侧又发明了第四种键形态（读侧按精确相等过滤）");
  }
  const writer = read(WRITER);
  if (writer === null) { return [...errs, `④ 读不到 ${WRITER} ⇒ 判据失效`]; }
  if (!writer.includes("dq_feedback_node_id(")) {
    errs.push("④ 反馈写侧未调用 dq_feedback_node_id ⇒ 写侧与读侧的键形态没有任何共同来源，迟早漂移");
  }
  return errs;
}

function run() {
  const rhaiSrc = readRel(RHAI);
  const rustSrc = readRel(RUST_REG);
  const tsSrc = readRel(TS_REG);
  if (rhaiSrc === null || rustSrc === null || tsSrc === null) {
    console.log("❌ 读不到源文件（rhai / Rust 登记表 / 前端镜像表）⇒ 判据失效");
    return 2;
  }
  const rhaiSlots = parseRhaiSlots(rhaiSrc);
  const rhaiHeader = parseRhaiHeader(rhaiSrc);
  const rust = parseRust(rustSrc);
  const ts = parseTs(tsSrc);
  if (!rust.ok || !ts.ok) {
    console.log(`❌ 解析失败: ${!rust.ok ? rust.reason : ts.reason}`);
    return 2;
  }
  const surface = surfaceCheck(rhaiSlots, rust.map, ts.map);
  if (surface.length) {
    surface.forEach((e) => console.log(`❌ ${e}`));
    return 2;
  }
  const problems = compareSlots(rhaiSlots, rhaiHeader, rust.map, ts.map);
  problems.push(...checkWriteSideKeyShape(readRel));

  console.log(
    `比对面：rhai 脚本体 ${rhaiSlots.length} 槽位 | rhai 头部注释 ${Object.keys(rhaiHeader).length} | Rust 登记表 ${Object.keys(rust.map).length} | 前端镜像表 ${Object.keys(ts.map).length}`,
  );
  if (problems.length === 0) {
    console.log("✅ 通过：三处槽位集合与配对一致、写侧 node_id 形态与唯一读侧闭环");
    return 0;
  }
  problems.forEach((p) => console.log(`✖ ${p}`));
  console.log("\n❌ 存在不一致（真相源 = harness::holding_period::DQ_ABBR_TO_BASE；rhai 头部注释是第三份登记，不是文档）");
  return 1;
}

/**
 * 正负对照：证明本门**会红**，而不是恒报绿。
 * 正控用「真实缺陷形态」在内存里复刻（缩写漂移 / 配对错位 / 少一侧 / 键形态改写），
 * 负控用磁盘上的当前形态（应无问题）。不改生产码，也不改磁盘文件。
 */
function selftest() {
  const rhaiSrc = fs.readFileSync(path.join(ROOT, RHAI), "utf8");
  const rustSrc = fs.readFileSync(path.join(ROOT, RUST_REG), "utf8");
  const tsSrc = fs.readFileSync(path.join(ROOT, TS_REG), "utf8");
  const rhaiSlots = parseRhaiSlots(rhaiSrc);
  const rhaiHeader = parseRhaiHeader(rhaiSrc);
  const rust = parseRust(rustSrc);
  const ts = parseTs(tsSrc);
  if (!rust.ok || !ts.ok) {
    console.log("SELFTEST FAIL：真实文件解析失败 ⇒ 无从对照");
    return 1;
  }
  const clone = (o) => structuredClone(o);
  const cases = [];

  cases.push({
    name: "负控 磁盘现状（三处一致）⇒ 应无问题",
    got: compareSlots(rhaiSlots, rhaiHeader, rust.map, ts.map).length,
    want: 0,
  });
  cases.push({
    name: "负控 磁盘现状（写侧键形态闭环）⇒ 应无问题",
    got: checkWriteSideKeyShape(readRel).length,
    want: 0,
  });

  // ① 集合漂移：前端表少一个缩写
  const tsDropped = clone(ts.map);
  delete tsDropped.lk;
  cases.push({
    name: "①正控 前端镜像表少一个缩写 ⇒ 必须报 1 处",
    got: compareSlots(rhaiSlots, rhaiHeader, rust.map, tsDropped).length,
    want: 1,
  });
  // ① 集合漂移：Rust 表多一个缩写 ⇒ 另外三处各报一次「缺槽位」（共 3 处）
  const rustExtra = clone(rust.map);
  rustExtra.ghost = "a-ghost-analyst";
  cases.push({
    name: "①正控 Rust 登记表多一个缩写 ⇒ 必须报 3 处（另外三侧各缺此槽位）",
    got: compareSlots(rhaiSlots, rhaiHeader, rustExtra, ts.map).length,
    want: 3,
  });
  // ② 配对错位：同一缩写在两侧指向不同节点 ⇒ 前端↔Rust、前端↔rhai头部各报一次（共 2 处）
  const tsSwapped = clone(ts.map);
  tsSwapped.mk = "a-fundamentals";
  cases.push({
    name: "②正控 同一缩写指向不同节点 ⇒ 必须报 2 处",
    got: compareSlots(rhaiSlots, rhaiHeader, rust.map, tsSwapped).length,
    want: 2,
  });
  // ② 配对错位：rhai 头部注释落后于 Rust 表 ⇒ 与 Rust、与前端各报一次（共 2 处）
  const headStale = clone(rhaiHeader);
  headStale.val = "a-value-investor";
  cases.push({
    name: "②正控 rhai 头部注释的节点 id 与 Rust 不一致 ⇒ 必须报 2 处",
    got: compareSlots(rhaiSlots, headStale, rust.map, ts.map).length,
    want: 2,
  });

  // ③ 扫描面失效
  cases.push({
    name: "③护栏 rhai 槽位解析为 0 ⇒ 必须报判据失效",
    got: surfaceCheck([], rust.map, ts.map).length,
    want: 1,
  });
  cases.push({
    name: "③护栏 Rust 登记表解析为 0 ⇒ 必须报判据失效",
    got: surfaceCheck(rhaiSlots, {}, ts.map).length,
    want: 1,
  });
  cases.push({
    name: "③护栏 前端镜像表解析为 0 ⇒ 必须报判据失效",
    got: surfaceCheck(rhaiSlots, rust.map, {}).length,
    want: 1,
  });

  // ④ 写侧键形态
  const fakeHolding = rustSrc.replace('base.strip_prefix("a-").unwrap_or(base)', "base");
  const fakeRead = (map) => (rel) => (rel in map ? map[rel] : null);
  cases.push({
    name: "④正控 写侧不再剥 `a-` 前缀 ⇒ 必须报 1 处",
    got: checkWriteSideKeyShape(fakeRead({ [RUST_REG]: fakeHolding, [WRITER]: "dq_feedback_node_id(" })).length,
    want: 1,
  });
  const fakeTier = rustSrc.replace(
    'base.strip_prefix("a-").unwrap_or(base)',
    'format!("{}--mid", base.strip_prefix("a-").unwrap_or(base))',
  );
  cases.push({
    name: "④正控 写侧给 node_id 加档后缀 ⇒ 必须报 1 处",
    got: checkWriteSideKeyShape(fakeRead({ [RUST_REG]: fakeTier, [WRITER]: "dq_feedback_node_id(" })).length,
    want: 1,
  });
  cases.push({
    name: "④正控 写侧绕过 dq_feedback_node_id ⇒ 必须报 1 处",
    got: checkWriteSideKeyShape(fakeRead({ [RUST_REG]: rustSrc, [WRITER]: "let node_id = base.to_string();" })).length,
    want: 1,
  });
  cases.push({
    name: "④护栏 登记表读不到 ⇒ 必须报判据失效（不能静默放行）",
    got: checkWriteSideKeyShape(fakeRead({ [WRITER]: "dq_feedback_node_id(" })).length,
    want: 1,
  });

  // 真实读数不是空表
  cases.push({
    name: "★真实读数：三处各 ≥ 10 槽位且集合非空（不是空表充当一致）",
    got: (rhaiSlots.length >= MIN_SLOTS && Object.keys(rust.map).length >= MIN_SLOTS
      && Object.keys(ts.map).length >= MIN_SLOTS) ? 1 : 0,
    want: 1,
  });

  let failed = 0;
  console.log("=== 门禁自检（正负对照）===");
  for (const c of cases) {
    const pass = c.got === c.want;
    if (!pass) { failed++; }
    console.log(`  ${pass ? "✅" : "❌"} ${c.name}（实得 ${c.got}，期望 ${c.want}）`);
  }
  console.log(failed === 0 ? "\nSELFTEST PASS" : `\nSELFTEST FAIL (${failed})`);
  return failed === 0 ? 0 : 1;
}

const argv = process.argv.slice(2);
if (process.argv[1] && process.argv[1].endsWith("check-dq-abbr-parity.mjs")) {
  process.exit(argv.includes("--selftest") ? selftest() : run());
}
