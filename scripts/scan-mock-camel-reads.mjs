#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * dev mock 的「camel 直读」普查器（任务 #211，从 #201 的一次性工件固化）。
 *
 * 背景：`src/lib/browserMock.ts` 的 `handleCommand` 在进 case 前先 `convertToSnakeCase(args)`
 * （**递归**）⇒ 任何从 `args` 派生的对象在读 `someCamelKey` 时恒 `undefined`。
 * 正确读法是既有 `readMockReqField(obj, "camelKey")`（camel 优先、snake 兜底）。
 *
 * 为什么要第二版：#201 第一版只覆盖「解构 / 直接成员访问」，漏掉**内联表达式**里的读点
 * （三元 `cond ? args?.fooBar : …`、`filter((x) => x.fooBar === args.targetId)` 回调内的外层读、
 * `obj["fooBar"]` 计算属性）。实测漏掉了 `list_memory_items` 的 `namespaceId`。
 *
 * 判据（机械、**按 case 块定作用域**——这一点是精度关键）：
 *   1. 把文件切成 `case "<cmd>": { … }` 块（按花括号深度）。
 *   2. *块内*固定点求「args 派生名」：`const X = <右侧提到 args 或已派生名>` 与
 *      `const { X } = args…`；再**减去**块内出现过的箭头参数 / 函数参数 / 对象键同名 ⇒ 防同名遮蔽。
 *   3. 扫 `BASE.prop` / `BASE?.prop` / `BASE["prop"]`：BASE ∈ 派生名且 prop 含大写字母 ⇒ 记一条。
 *      单字键（`id` / `title` / `content`）`camelToSnake` 后逐字相同，**不是**死点。
 *
 * 为什么必须按块作用域：第一版用**文件级**集合，`p` / `title` / `input` 这类常见名在别处
 * 一旦绑定过就被全局当成 args 派生 ⇒ 112 条命中里九成是箭头参数与本地字符串变量（实测）。
 *
 * 已知边界（别把「零命中」当零风险）：
 *   - 只做**块内**数据流；跨 case / 跨模块传进来的派生对象判不出；
 *   - 只剥行注释；块注释里的同名访问会误计（宁多勿漏）；
 *   - 不解析类型。
 *
 * 用法：
 *   node scripts/scan-mock-camel-reads.mjs             # 扫真实文件（命中 > 0 ⇒ 退 1）
 *   node scripts/scan-mock-camel-reads.mjs --selftest  # 内建正负对照（零盘扫描）
 *   node scripts/scan-mock-camel-reads.mjs --json      # 机器可读
 */

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const TARGET = join(HERE, "..", "src", "lib", "browserMock.ts");

/** 只剥行注释（行内 `//` 前引号成对才算注释；宁可漏剥，不可把字符串里的 // 当注释）。 */
export function stripLineComments(src) {
  return src
    .split("\n")
    .map((line) => {
      const i = line.indexOf("//");
      if (i < 0) return line;
      const head = line.slice(0, i);
      const quotes = (head.match(/"/g) ?? []).length + (head.match(/'/g) ?? []).length;
      return quotes % 2 === 0 ? head : line;
    })
    .join("\n");
}

/** 把源码切成 `{ name, startLine, lines[] }` 的 case 块。 */
export function splitCaseBlocks(src) {
  const code = stripLineComments(src);
  const lines = code.split("\n");
  const blocks = [];
  let cur = null;
  let depth = 0;
  lines.forEach((line, idx) => {
    const m = /^\s*case\s+["']([^"']+)["']\s*:\s*\{?\s*$/.exec(line);
    if (m) {
      cur = { name: m[1], startLine: idx + 1, lines: [line] };
      blocks.push(cur);
      depth = (line.match(/\{/g) ?? []).length - (line.match(/\}/g) ?? []).length;
      return;
    }
    if (cur) {
      cur.lines.push(line);
      depth += (line.match(/\{/g) ?? []).length - (line.match(/\}/g) ?? []).length;
      if (depth <= 0) cur = null;
    }
  });
  return blocks;
}

/** 块内求「args 派生名」，再减去块内的参数 / 键同名（防遮蔽）。
 *
 * 派生规则刻意**只认直投影**（这是精度关键 —— 上一版「右侧只要提到派生名就算」把
 * `const lead = MOCK_LEADS.find(…leadId…)` 也算派生，一次扫出 75 条里九成是数据行/值变量）：
 *   - 右侧整体就是载体本身：`args` / `input` / `req`（括号、`as T`、`?? {}` 可剥）；
 *   - `载体.input` / `载体.request` / `载体.args` / `载体.payload`（同样可带 `as T`/`?? {}`）；
 *   - `readMockReqField(...)` —— 它返回的是嵌套对象（如 `input` / `request`），本身即载体；
 *   - 解构 `const { input, request } = 载体` 只取**载体名集合内**的键（`id` / `title` 是值不是载体）。
 */
const CARRIER_KEYS = new Set(["input", "request", "args", "payload"]);
export function deriveArgsNames(blockLines) {
  const code = blockLines.join("\n");
  const derived = new Set(["args"]);
  const strip = (rhs) =>
    rhs
      .replace(/^\s*\(/, "")
      .replace(/\)\s*$/, "")
      .replace(/\s+as\s+[^;]+$/, "")
      .trim();
  const directCarrier = (rhs, set) => {
    const t = strip(rhs);
    // `X ?? {}` / `X ?? Y`：取 `??` 左侧再判
    const left = t.split("??")[0].trim();
    if (/^readMockReqField\b/.test(left)) return true;
    const m = /^([A-Za-z_$][\w$]*)(?:\s*\??\.\s*([A-Za-z_$][\w$]*))?$/.exec(strip(left));
    if (!m) return false;
    const [, base, prop] = m;
    if (!set.has(base)) return false;
    return prop === undefined || CARRIER_KEYS.has(prop);
  };
  const namesOf = (lhs) =>
    lhs.startsWith("{")
      ? lhs
          .slice(1, -1)
          .split(",")
          .map((s) => s.split(":")[0].split("=")[0].trim())
          .filter((s) => CARRIER_KEYS.has(s))
      : [lhs.trim()];
  const bindingRe = /\b(?:const|let|var)\s+(\{[^}]*\}|[A-Za-z_$][\w$]*)\s*(?::[^=]+)?=\s*([^;\n]*)/g;
  let grew = true;
  while (grew) {
    grew = false;
    bindingRe.lastIndex = 0;
    let m;
    while ((m = bindingRe.exec(code))) {
      const [, lhs, rhs] = m;
      if (!directCarrier(rhs, derived)) continue;
      for (const name of namesOf(lhs)) {
        if (name && !derived.has(name)) {
          derived.add(name);
          grew = true;
        }
      }
    }
  }
  // 遮蔽面：箭头参数 `(p) =>` / `p =>`、函数参数 `function f(a, b)`、对象键 `p: …`
  const shadow = new Set();
  for (const m of code.matchAll(/\(\s*([^()]*)\s*\)\s*=>/g)) {
    for (const n of m[1].split(",")) shadow.add(n.split("=")[0].trim());
  }
  for (const m of code.matchAll(/\b([A-Za-z_$][\w$]*)\s*=>/g)) shadow.add(m[1]);
  for (const m of code.matchAll(/function\s+[A-Za-z_$][\w$]*\s*\(([^)]*)\)/g)) {
    for (const n of m[1].split(",")) shadow.add(n.split("=")[0].split(":")[0].trim());
  }
  for (const n of shadow) derived.delete(n);
  return derived;
}

const camelToSnake = (s) => s.replace(/[A-Z]/g, (c) => `_${c.toLowerCase()}`);

/** 扫一个 case 块里的 camel 直读。返回 [{ name, line, base, prop, text, covered }]。
 *
 * 两条降噪判据（都有正负控）：
 *   - **方法调用不算读键**：`x.startsWith(` / `x.toLowerCase(` 的 camel 部分是方法名 ⇒ 跳过；
 *   - **camel 失败但有 snake 兜底**：同一行出现 `BASE.snake_key` 或 `BASE["snake_key"]`
 *     （即 `x.fleetId ?? x.fleet_id` 这种「camel 优先、snake 兜底」写法）⇒ 标 `covered: true`，
 *     **算能跑通**，不进死点计数（但仍在 --json 里列出，便于人工复核兜底方向对不对）。
 */
export function scanCamelReadsInBlocks(src) {
  const out = [];
  for (const block of splitCaseBlocks(src)) {
    const derived = deriveArgsNames(block.lines);
    block.lines.forEach((line, idx) => {
      for (const base of derived) {
        const dot = new RegExp(`\\b${base}\\s*\\??\\s*\\.\\s*([A-Za-z_$][\\w$]*)(\\s*\\()?`, "g");
        const bracket = new RegExp(`\\b${base}\\s*\\??\\s*\\[\\s*["']([A-Za-z_$][\\w$]*)["']\\s*\\]`, "g");
        for (const re of [dot, bracket]) {
          let m;
          while ((m = re.exec(line))) {
            const [, prop, callParen] = m;
            if (!/[A-Z]/.test(prop)) continue; // 单字键 snake 化后逐字相同
            if (callParen) continue; // 方法调用（startsWith / toLowerCase …），不是读键
            const snake = camelToSnake(prop);
            const snakeRe = new RegExp(
              `\\b${base}\\s*\\??\\s*(?:\\.\\s*${snake}\\b|\\[\\s*["']${snake}["']\\s*\\])`,
            );
            out.push({
              name: block.name,
              line: block.startLine + idx,
              base,
              prop,
              text: line.trim().slice(0, 160),
              covered: snakeRe.test(line),
            });
          }
        }
      }
    });
  }
  return out;
}

function selftest() {
  const cases = [];
  const check = (name, cond, detail = "") => cases.push({ name, ok: !!cond, detail });
  const scan = (s) => scanCamelReadsInBlocks(s);

  let h = scan(`case "a": {\n  const x = args.fooBar;\n  return x;\n}`);
  check("正控：args.fooBar 必须命中", h.length === 1 && h[0].prop === "fooBar", JSON.stringify(h));

  h = scan(`case "b": {\n  const { input } = args as { input: X };\n  const v = input.innerKey;\n  return v;\n}`);
  check("正控：`const { input } = args` 派生后 innerKey 必须命中", h.some((x) => x.prop === "innerKey"), JSON.stringify(h));

  h = scan(`case "c": {\n  const a = args?.wikiId ? String(args?.wikiId) : "";\n  return a;\n}`);
  check("正控：三元内联 + 可选链必须命中两次", h.filter((x) => x.prop === "wikiId").length === 2, JSON.stringify(h));

  h = scan(`case "d": {\n  arr.filter((x) => x.id === args.targetId);\n}`);
  check("正控：回调内读外层 args 必须命中", h.some((x) => x.prop === "targetId"), JSON.stringify(h));

  h = scan(`case "e": {\n  const v = args["someKey"];\n  return v;\n}`);
  check("正控：args[\"someKey\"] 必须命中", h.some((x) => x.prop === "someKey"), JSON.stringify(h));

  h = scan(`case "f": {\n  const t = args.title;\n  return t;\n}`);
  check("负控：单字键 args.title 不得命中", h.length === 0, JSON.stringify(h));

  h = scan(`case "g": {\n  const rows = getStore<Row[]>("rows", []);\n  const v = rows[0].conversationId;\n  return v;\n}`);
  check("负控：store 行上的 camel 读不得命中", h.length === 0, JSON.stringify(h));

  h = scan(`case "h": {\n  const v = readMockReqField<string>(args, "someKey");\n  return v;\n}`);
  check("负控：readMockReqField 的第二实参不得命中", h.length === 0, JSON.stringify(h));

  h = scan(`case "i": {\n  // legacy: args.oldCamelKey\n  const t = args.title;\n  return t;\n}`);
  check("负控：行注释里的读点不得命中", h.length === 0, JSON.stringify(h));

  // 块作用域负控（第一版 112 条假阳性的主因）：A 块里绑定过 `p`，B 块的箭头参数 `p` 不得被牵连
  h = scan(`case "j": {\n  const p = args.programName;\n  return p;\n}\ncase "k": {\n  rows.find((p) => p.capabilityId === x);\n}`);
  check(
    "负控：跨 case 同名不得牵连（块作用域）",
    h.length === 1 && h[0].prop === "programName",
    JSON.stringify(h),
  );

  // 函数参数遮蔽负控：`input` 作为顶层函数参数时，函数体里的 `input.someKey` 不是 args 派生
  h = scan(`function camelToSnake(input: string) { return input.someKey; }\ncase "l": {\n  const t = args.title;\n  return t;\n}`);
  check("负控：函数参数同名不得命中", h.length === 0, JSON.stringify(h));

  // 负控 5：方法调用（camel 方法是方法名，不是键）。用**单字键**做基座，
  // 这样唯一可能被误记的就是 `startsWith` 本身 —— 正是本条要排除的对象。
  h = scan(`case "m": {\n  const t = args.token.startsWith("x");\n  return t;\n}`);
  check("负控：方法调用 args.token.startsWith 不得命中（只记读键）", h.length === 0, JSON.stringify(h));

  // 负控 6：camel 死读但有 snake 兜底 ⇒ 标 covered，不算死点
  h = scan(`case "n": {\n  const v = (args.fleetId ?? args.fleet_id ?? "") as string;\n  return v;\n}`);
  check(
    "负控：camel+snake 兜底应标 covered 且不算死点",
    h.length === 1 && h[0].covered === true,
    JSON.stringify(h),
  );

  // 正控补：同样的 camel 读**没有**兜底 ⇒ covered=false（真死点）
  h = scan(`case "o": {\n  const v = args.fleetId;\n  return v;\n}`);
  check("正控：无兜底的 camel 读 covered=false", h.length === 1 && h[0].covered === false, JSON.stringify(h));

  const failed = cases.filter((c) => !c.ok);
  for (const c of cases) console.log(`${c.ok ? "  ok" : "✖ FAIL"}  ${c.name}${c.ok ? "" : ` —— ${c.detail}`}`);
  console.log(`\nSELFTEST ${failed.length ? "FAILED" : "OK"}：${cases.length - failed.length}/${cases.length} 条通过（正控 6 / 负控 6）`);
  return failed.length === 0;
}

const isMain = process.argv[1] && process.argv[1].replace(/\\/g, "/").endsWith("scripts/scan-mock-camel-reads.mjs");
if (isMain) {
  if (process.argv.includes("--selftest")) process.exit(selftest() ? 0 : 2);
  let src;
  try {
    src = readFileSync(TARGET, "utf8");
  } catch (e) {
    console.error(`✖ 读不到扫描面 ${TARGET}：${e.message}`);
    process.exit(2);
  }
  if (src.trim().length === 0) {
    console.error("✖ 扫描面为空 —— 拒绝把「零命中」当结论");
    process.exit(2);
  }
  const blocks = splitCaseBlocks(src);
  const hits = scanCamelReadsInBlocks(src);
  const dead = hits.filter((h) => !h.covered);
  const covered = hits.filter((h) => h.covered);
  if (process.argv.includes("--json")) {
    console.log(JSON.stringify({ target: TARGET, caseBlocks: blocks.length, dead, covered }, null, 1));
  } else {
    console.log(`扫描面：${TARGET}（${src.split("\n").length} 行 / ${blocks.length} 个 case 块）`);
    console.log(`\n[死点 ${dead.length}] camel 读且无 snake 兜底 —— 这些才是恒 undefined：`);
    for (const x of dead) console.log(`  ${x.line}\t${x.name}\t${x.base}.${x.prop}\t${x.text}`);
    console.log(`\n[有兜底 ${covered.length}] 同一行已有 snake 兜底（能跑通，建议仍改成 readMockReqField 统一）：`);
    for (const x of covered) console.log(`  ${x.line}\t${x.name}\t${x.base}.${x.prop}\t${x.text}`);
    console.log(`\n死点 ${dead.length} 处（>0 ⇒ 退 1）；有兜底 ${covered.length} 处。`);
  }
  process.exit(dead.length === 0 ? 0 : 1);
}
