#!/usr/bin/env node
/**
 * `.rhai` 决策脚本「数值判据形态」门禁（`PLAN-four-horizon-workflow-alignment.md` §五十二 ③）。
 *
 * ## 为什么需要它
 *
 * JSON 数字落进 Rhai 有**两条形态不同的注入通道**，`cargo check` / 编译门全绿也照样存在这条缺陷：
 *
 * | 通道 | 代码位置 | 整数的落点 |
 * |---|---|---|
 * | 顶层**标量** number 参数 | `src-tauri/crates/rt-workflow/src/work_engine/executors/code_executor.rs` 的 `Value::Number` 分支（先 `as_f64`） | f64 |
 * | 其余一切（顶层 map/array 参数的**每一层**、`json_parse()` 的产物） | `src-tauri/crates/harness/src/rhai_engine.rs` 的 `json_value_to_dynamic`（`as_i64` 优先） | **i64** |
 *
 * 于是 `type_of(x) == "f64"` 只在第一种形态下等价于「x 是个数」。拿它当嵌层值的数值判据 ⇒
 * **整数被静默判成缺席**（2026-10-04 实证：分支表 `days`/`volLookbackDays` 是整数 ⇒ 四档价带恒 0）。
 * 同理，64-bit 构建下 `type_of` 对整数给 `"i64"`，**`"int"` 那个分支恒不成立** —— 写着它不等于收着整数。
 *
 * | 门 | 锁什么 | 必须对它报红的历史形态 |
 * |---|---|---|
 * | **a 嵌层数值判据必须走桥** | 操作数是嵌层读取（含 `[` 或 `.`）的 `type_of(..) == "f64"` 单型守卫，必须已经过 `num_of(` 或同句 `.to_float()` | `portfolio-mgr.rhai` 筹码面五处（`t["shares"]` / `e["buyAmount"]` … 整数 ⇒ 恒 0 兜底）、`pace-calc.rhai` 十处（LLM 写 `confidence: 85` ⇒ 落 0.5，用近似值顶替） |
 * | **b `"int"` 死分支** | `== "int"` / `!= "int"` 且同句无 `"i64"`、无 `num_of(` | `pace-calc.rhai:159/325/333`、`consistency-check.rhai:69`（看着像收了整数，实际恒假） |
 * | **c 桥的唯一权威源** | `num_of` 必须在 harness 的 `register_common_functions` 里注册（两个元数），脚本侧不得再 `fn num_of` | 同一语义曾有 3 份脚本内定义（`portfolio-mgr` / `risk-level` / `trader-proxy`），新脚本再加一份就是第四个 |
 *
 * **扫描面非零**是第 0 条判据：任何一门解析到 0 命中即按假绿处理（判据同 `check-reco-horizon-parity.mjs`）。
 *
 * 用法：
 *   node scripts/check-rhai-numeric-typing.mjs            # 比对（有违规 exit 1）
 *   node scripts/check-rhai-numeric-typing.mjs --selftest # 正负对照（复刻修复前形态，证明它会红）
 */

import fs from "node:fs";
import path from "node:path";
import process from "node:process";

const ROOT = path.resolve(import.meta.dirname, "..");
const RHAI_DIR = "src-tauri/src/commands";
const HARNESS_ENGINE = "src-tauri/crates/harness/src/rhai_engine.rs";

const read = (rel) => fs.readFileSync(path.join(ROOT, rel), "utf8");

/** 递归列出待检的 `.rhai`（含 `opc_setup/` 等子目录）。 */
function listRhai(dirRel) {
  const out = [];
  const walk = (abs, rel) => {
    for (const ent of fs.readdirSync(abs, { withFileTypes: true })) {
      const absChild = path.join(abs, ent.name);
      const relChild = `${rel}/${ent.name}`;
      if (ent.isDirectory()) { walk(absChild, relChild); }
      else if (ent.name.endsWith(".rhai")) { out.push({ name: relChild, src: read(relChild) }); }
    }
  };
  walk(path.join(ROOT, dirRel), dirRel);
  return out.sort((a, b) => (a.name < b.name ? -1 : 1));
}

/**
 * 剥掉行注释，返回**只含代码**的行数组（保留行号 1 基）。
 *
 * 为什么不能简单 `indexOf("//")`：Rhai 字符串/反引号模板里可以合法出现 `//`
 * （URL、路径），一刀切会把代码拦腰截断 ⇒ 门自己失真。这里按引号状态扫。
 * 块注释只处理脚本里实际使用的「起于斜杠星、闭合于星斜杠」的单行形态；跨行块注释在本仓
 * `.rhai` 中未出现，若将来出现，`selftest` 的「注释不得被吃掉」自证会先红，不会静默漏检。
 */
function codeLines(src) {
  return src.split("\n").map((line) => {
    const trimmed = line.trim();
    if (trimmed.startsWith("//") || trimmed.startsWith("/*") || trimmed.startsWith("*")) { return ""; }
    let inDouble = false;
    let inBacktick = false;
    let cut = line.length;
    for (let i = 0; i < line.length; i += 1) {
      const c = line[i];
      if (c === "\\" && (inDouble || inBacktick)) { i += 1; continue; }
      if (c === '"' && !inBacktick) { inDouble = !inDouble; continue; }
      if (c === "`" && !inDouble) { inBacktick = !inBacktick; continue; }
      if (!inDouble && !inBacktick && c === "/" && line[i + 1] === "/") { cut = i; break; }
    }
    return line.slice(0, cut);
  });
}

/** `type_of( … ) == "f64"` / `!= "f64"`，捕获括号内的操作数（允许一层嵌套括号）。 */
const TYPE_OF_F64 = /type_of\(([^()]*(?:\([^()]*\)[^()]*)*)\)\s*[!=]=\s*"f64"/g;
const OPERAND_NESTED = /[[.]/;

/** 门 a：嵌层读取上的 f64 单型守卫必须已经过桥。 */
function checkNestedF64GuardMustBeBridged(files) {
  const violations = [];
  let scanned = 0;
  for (const { name, src } of files) {
    codeLines(src).forEach((line, i) => {
      if (!line.includes('== "f64"') && !line.includes('!= "f64"')) { return; }
      for (const m of line.matchAll(TYPE_OF_F64)) {
        scanned += 1;
        const operand = m[1];
        if (!OPERAND_NESTED.test(operand)) { continue; } // 顶层标量：f64 判据成立，不在本门面积
        const rest = line.slice(m.index + m[0].length);
        const bridged = line.includes("num_of(") || rest.includes(".to_float()");
        if (!bridged) {
          violations.push(`${name}:${i + 1} 嵌层判据未走桥 → ${line.trim()}`);
        }
      }
    });
  }
  return { violations, scanned };
}

/** 门 b：`"int"` 单型比较是死分支（64-bit 构建下 `type_of` 给 `"i64"`）。 */
function checkDeadIntBranch(files) {
  const violations = [];
  let scanned = 0;
  for (const { name, src } of files) {
    codeLines(src).forEach((line, i) => {
      if (!/[!=]=\s*"int"/.test(line)) { return; }
      scanned += 1;
      if (line.includes('"i64"') || line.includes("num_of(")) { return; } // 成对兜桥 = 冗余但无害
      violations.push(`${name}:${i + 1} 死分支 == "int" → ${line.trim()}`);
    });
  }
  return { violations, scanned };
}

/** 门 c：`num_of` 的唯一权威源在 harness，脚本侧不得再定义。 */
function checkBridgeSingleSource(files, harnessSrc) {
  const violations = [];
  let scanned = 0;
  const oneArg = /register_fn\(\s*"num_of"\s*,\s*\|x: rhai::Dynamic\|/;
  const twoArg = /"num_of",\s*\n?\s*\|x: rhai::Dynamic, dflt: f64\|/;
  if (!oneArg.test(harnessSrc)) { violations.push(`${HARNESS_ENGINE} 未注册 num_of(x)（1 元）`); }
  if (!twoArg.test(harnessSrc)) { violations.push(`${HARNESS_ENGINE} 未注册 num_of(x, dflt)（2 元）`); }
  scanned += 2;
  for (const { name, src } of files) {
    codeLines(src).forEach((line, i) => {
      if (!/^\s*fn\s+num_of\b/.test(line)) { return; }
      scanned += 1;
      violations.push(`${name}:${i + 1} 脚本内重复定义 num_of ⇒ 权威源必须在 harness`);
    });
  }
  return { violations, scanned };
}

// ────────────────────────────────────────────────────────────────────────────
// selftest：正负对照。坏样本必须红、好样本必须绿 —— 否则「门在绿」不构成证据。
// ────────────────────────────────────────────────────────────────────────────
function selftest() {
  const bad = [{
    name: "bad.rhai",
    src: [
      `let vol = if type_of(t["shares"]) == "f64" { t["shares"] } else { 0.0 };`,
      `let c = if type_of(inner["confidence"]) == "f64" { 1.0 } else if type_of(inner["confidence"]) == "int" { 2.0 } else { 0.5 };`,
      `fn num_of(x) { x }`,
      `// 注释里的 type_of(cfg["x"]) == "f64" 与 == "int" 都不该被计数`,
    ].join("\n"),
  }];
  const good = [{
    name: "good.rhai",
    src: [
      `let vol = num_of(t["shares"], 0.0);`,
      `let raw = num_of(inner["confidence"]); if type_of(raw) == "f64" { raw / 100.0 } else { 0.5 };`,
      `let pe = if type_of(fin.pe_ttm) == "f64" { fin.pe_ttm } else { fin.pe_ttm.to_float() };`,
      `let n = if type_of(market_regime_prior) == "f64" { market_regime_prior } else { () };`,
      `let ok = type_of(x) == "i64" || type_of(x) == "int";`,
    ].join("\n"),
  }];

  const a = checkNestedF64GuardMustBeBridged(bad);
  const b = checkDeadIntBranch(bad);
  const c = checkBridgeSingleSource(bad, "");
  const ga = checkNestedF64GuardMustBeBridged(good);
  const gb = checkDeadIntBranch(good);
  const gc = checkBridgeSingleSource(good, read(HARNESS_ENGINE));

  const expect = (label, cond) => {
    if (cond) { console.log(`OK  selftest ${label}`); }
    else { console.log(`FAIL selftest ${label}`); process.exitCode = 1; }
  };
  expect("a 坏样本必须红（两处嵌层未桥）", a.violations.length === 2);
  expect("b 坏样本必须红（== \"int\" 恒假）", b.violations.length === 1);
  expect("c 脚本内定义 num_of 必须红", c.violations.length === 3); // 两个元数缺失 + 一处重复定义
  expect("剥注释不得吃掉代码", a.scanned === 2 && b.scanned === 1);
  expect("好样本必须绿", ga.violations.length === 0 && gb.violations.length === 0
    && gc.violations.length === 0);
  expect("好样本扫描面非零", ga.scanned > 0 && gb.scanned > 0 && gc.scanned > 0);
  process.exit(process.exitCode || 0);
}

function main() {
  const files = listRhai(RHAI_DIR);
  if (files.length === 0) {
    console.log(`BROKEN：${RHAI_DIR} 下没找到 .rhai ⇒ 扫描面为 0`);
    process.exit(2);
  }
  const gates = [
    ["a 嵌层数值判据必须走 num_of 桥", checkNestedF64GuardMustBeBridged(files)],
    ["b 死分支 == \"int\"", checkDeadIntBranch(files)],
    ["c num_of 唯一权威源在 harness", checkBridgeSingleSource(files, read(HARNESS_ENGINE))],
  ];

  let red = 0;
  for (const [label, res] of gates) {
    if (res.scanned === 0) {
      console.log(`BROKEN ${label}：扫描面为 0 ⇒ 判据失效（按假绿处理）`);
      red += 1;
      continue;
    }
    if (res.violations.length > 0) {
      console.log(`RED ${label}（扫描面 ${res.scanned}）：`);
      for (const v of res.violations) { console.log(`  - ${v}`); }
      red += res.violations.length;
    } else {
      console.log(`OK  ${label}（扫描面 ${res.scanned}）`);
    }
  }
  if (red > 0) {
    console.log(`\n结论: ${red} 处违规/失效`);
    process.exit(1);
  }
  console.log(`\n结论: ${files.length} 个 .rhai 的数值判据形态一致（桥唯一、嵌层已桥接、无 "int" 死分支）`);
}

if (process.argv.includes("--selftest")) { selftest(); } else { main(); }
