#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
//
// 端口公理**写路径接线守卫**（C1 P0，2026-09-14）。
//
// # 为什么需要它
//
// 门禁（`harness::workflow_port_axioms::enforce_port_axioms`）只在**被调用到**时才生效。
// 写路径有十几条，分散在 DAO / 命令层 / 种子文件里 —— 新增一条写入路径（新 seed、
// 新导入、新 AI 生成）而忘了接门禁，**没有任何编译错误**会提示，模板照样落库。
// `prod-startup-mvp` 的 `s-gonogo` 死链就是这么活过整个门禁升级的。
//
// # 枚举模式（必须多条，缺一条就漏扫 —— 这是本脚本第一版踩过的坑）
//
// | 模式 | 覆盖 | 举例 |
// |---|---|---|
// | A `workflow_template::ActiveModel {` | 整行 insert / upsert | `seed_production.rs` |
// | B `workflow_template::Column::Nodes`/`Edges` | `update_many().col_expr(..)` | `opc_workflows/mod.rs` 回填 |
// | C `workflow_template::Entity::insert/update_many` | 用 helper 构造 ActiveModel 后直接落库 | — |
// | D 裸 `ActiveModel {`（文件内 `use ...workflow_template::ActiveModel`） | 同上但省略前缀 | `stock_analysis.rs` |
//
// 第一版只有 A + B：漏掉了 D（`stock_analysis.rs` 用裸 `ActiveModel {`），
// 而「只看 B」时又只找到 3 个文件 —— 单模式枚举必漏。
//
// # 判定：**函数级**，且「已接」有两条合法途径
//
// 1. 函数体内出现门禁标记（`enforce_port_axioms` / `warn_port_axioms` / `is_port_axiom_clean` /
//    `port_axiom_errors`）；
// 2. 函数体内调用 DAO 写函数（`insert|upsert|update_workflow_template`）—— 门禁在 DAO 里，
//    这是**收口下游**，与 ① 等价。
//
// 途径 2 不可省：`skill_decomposition.rs` / `skill_workflow.rs` / `workflow_ai/compile.rs` 等
// 都经 DAO 落库，按「文件里有没有门禁字样」判会把它们全误报成未接。
//
// # 两道防自欺
//
// - **剥注释与字符串后再匹配**。第一版直接匹配原文，结果被我写在 `opc_workflows/mod.rs`
//   注释里的「判据复用 `port_axiom_errors`」骗过，把它报成已接（实际没接）。
// - **DAO 写函数自身的定义处只认途径 ①**。否则函数名 `insert_workflow_template` 会命中
//   途径 ② 的正则，等于自己证明自己。
//
// 退出码：0 = 全部已接；1 = 有未接点；2 = 扫描失败。

import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join, relative, sep } from "node:path";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = join(HERE, "..");
const SCAN_ROOTS = [join(REPO, "src-tauri", "src"), join(REPO, "src-tauri", "crates")];

const WRITE_PATTERNS = [
  {
    id: "A",
    name: "ActiveModel 构造（带 workflow_template:: 前缀）",
    re: /workflow_template::ActiveModel\s*\{/,
  },
  {
    id: "B",
    name: "列级写（update_many nodes/edges）",
    re: /workflow_template::Column::(Nodes|Edges)\b/,
  },
  {
    id: "C",
    name: "Entity::insert 直接落库（整行写）",
    re: /workflow_template::Entity::insert\b/,
  },
  {
    id: "E",
    name: "Entity::update_many 按列写（需同函数出现模式 B）",
    re: /workflow_template::Entity::update_many\b/,
    requiresPattern: "B",
  },
  {
    id: "D",
    name: "ActiveModel 构造（裸名，需同文件 use）",
    re: /(?:^|[^:A-Za-z_])ActiveModel\s*\{/,
    needsFileContent: /use\s+[\w:]*workflow_template::(?:\{[^}]*\bActiveModel\b|ActiveModel)/,
  },
];

const GATE_MARKERS = [
  /enforce_port_axioms/,
  /warn_port_axioms/,
  /is_port_axiom_clean/,
  /port_axiom_errors/,
];

/**
 * 模式 C/E 的「整行写 vs 按列写」区分是必要的：`Entity::update_many` 也用于只改
 * `route_path` / `variables` 的场合（`backfill_missing_route_paths`、
 * `apply_reco_weights`）—— 那些函数**不写 nodes/edges**，要求它们接门禁是假阳性。
 * `Entity::insert` 则一律是整行写入，必定带 nodes/edges。
 */
const PATTERN_REQUIRES = { E: "B" };

/**
 * 构造辅助：把 `WorkflowTemplateData` 转成 `ActiveModel` 的函数。它们**自身不是落库点**，
 * 无需门禁；「用它们构造后直接落库」的调用点会被模式 C 抓到，所以豁免不会留盲区。
 */
const CONSTRUCTOR_HELPERS = new Set([
  "build_active_model_from_data",
  "data_to_active_model",
  "wtd_to_active_model",
  "model_to_active_model",
]);

/** 途径 ②：经这些 DAO 写函数落库 = 已被其内部的门禁覆盖。 */
const DAO_WRITE_CALLS = [
  /\binsert_workflow_template\s*\(/,
  /\bupsert_workflow_template\s*\(/,
  /\bupdate_workflow_template\s*\(/,
];

/** DAO 写函数自身的名字 —— 它们出现在自己的定义处时不算「途径 ②」。 */
const DAO_WRITE_FN_NAMES = new Set([
  "insert_workflow_template",
  "upsert_workflow_template",
  "update_workflow_template",
]);

/**
 * 把**注释与字符串字面量**替换成等长空白（换行原样保留），使行号与原文一一对应。
 *
 * 必须连字符串一起剥：Rust 里 `"/*"`、`"//"` 之类很常见（路径 glob / 正则 / URL），
 * 只剥注释会让状态机在字符串里读到 `/*` 而进入块注释态，**吞掉后面成百行代码**
 * —— 那是漏扫（致命）。第一版就是这么把 `seed_stock_analysis.rs` 的写入点整个吞掉的。
 *
 * 剥字符串则要正确处理 `'a` lifetime / `'x'` char / `r#"..."#` 原始字符串，否则
 * 同样会吞代码。这里：
 * - char 字面量按**码点长度**判定（`'中'` 是 3 个 UTF-16 码元），判不出来就当 lifetime
 *   —— 当 lifetime 是安全侧（不吞代码）；
 * - 原始字符串 `r"…"` / `r#"…"#` / `br#"…"#` 按 `"` + 同数量 `#` 找闭合。
 *
 * 等长性由调用点的行数守恒断言保证。
 */
function sanitize(text) {
  let out = "";
  let i = 0;
  const n = text.length;
  let state = "code";
  const skip = (k) => {
    for (let t = 0; t < k && i < n; t++) {
      out += text[i] === "\n" ? "\n" : " ";
      i++;
    }
  };
  while (i < n) {
    const c = text[i];
    const c2 = text[i + 1];
    if (state === "code") {
      const raw = text.slice(i, i + 4).match(/^(?:b?r)(#*)"/);
      if (raw) {
        const closer = '"' + raw[1];
        const end = text.indexOf(closer, i + raw[0].length);
        const endIdx = end < 0 ? n : end + closer.length;
        skip(endIdx - i);
        continue;
      }
      if (c === "/" && c2 === "/") {
        state = "line";
        skip(2);
        continue;
      }
      if (c === "/" && c2 === "*") {
        state = "block";
        skip(2);
        continue;
      }
      if (c === '"') {
        state = "str";
        skip(1);
        continue;
      }
      if (c === "'") {
        let len = 0;
        if (text[i + 1] === "\\") {
          const m = text.slice(i + 1).match(/^\\(?:u\{[0-9a-fA-F_]{1,6}\}|x[0-9a-fA-F]{2}|.)/);
          if (m && text[i + 1 + m[0].length] === "'") len = m[0].length + 2;
        } else {
          const cp = text.codePointAt(i + 1);
          const cl = cp === undefined ? 0 : cp > 0xffff ? 2 : 1;
          if (cl && text[i + 1 + cl] === "'") len = cl + 2;
        }
        if (len) {
          skip(len);
          continue;
        }
        out += c;
        i++;
        continue;
      }
      out += c;
      i++;
      continue;
    }
    if (state === "line") {
      out += c === "\n" ? "\n" : " ";
      if (c === "\n") state = "code";
      i++;
      continue;
    }
    if (state === "block") {
      if (c === "*" && c2 === "/") {
        state = "code";
        skip(2);
        continue;
      }
      out += c === "\n" ? "\n" : " ";
      i++;
      continue;
    }
    // str
    if (c === "\\") {
      skip(2);
      continue;
    }
    if (c === '"') {
      state = "code";
      skip(1);
      continue;
    }
    out += c === "\n" ? "\n" : " ";
    i++;
  }
  return out;
}

function walk(dir, out = []) {
  let entries;
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch {
    return out;
  }
  for (const e of entries) {
    if (e.name === "target" || e.name === "node_modules" || e.name.startsWith(".")) continue;
    const p = join(dir, e.name);
    if (e.isDirectory()) walk(p, out);
    else if (e.name.endsWith(".rs")) out.push(p);
  }
  return out;
}

const FN_RE = /^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:async\s+)?fn\s+([A-Za-z0-9_]+)/;

/**
 * 命中行所属函数的 [start, end] 与函数名（基于净化后的行）。
 *
 * ⚠ 必须**逐候选校验区间包含性**：Rust 允许函数内嵌套 `fn`（`seed_stock_analysis.rs`
 * 就在 `seed_stock_analysis_workflow_template` 内部定义了
 * `fn adjust_positions_to_relative`）。只取「往上第一个 fn」会把命中行判给那个
 * 早已结束的内层函数 ⇒ 门禁加在外层也报「未接」（假红），或反过来给它一个错误的宿主。
 */
function enclosingFunction(lines, hitIdx) {
  for (let i = hitIdx; i >= 0; i--) {
    const m = lines[i].match(FN_RE);
    if (!m) continue;
    const end = matchBraceEnd(lines, i);
    if (end >= hitIdx) {
      return { name: m[1], start: i, end, text: lines.slice(i, end + 1).join("\n") };
    }
  }
  return { name: "<top-level>", start: hitIdx, end: hitIdx, text: "" };
}

/** 从 `startLine` 起按花括号配平，返回闭合行；找不到则返回最后一行。 */
function matchBraceEnd(lines, startLine) {
  let depth = 0;
  let opened = false;
  for (let i = startLine; i < lines.length; i++) {
    for (const ch of lines[i]) {
      if (ch === "{") {
        depth++;
        opened = true;
      } else if (ch === "}") {
        depth--;
      }
    }
    if (opened && depth <= 0) return i;
  }
  return lines.length - 1;
}

/** 「该函数是否已接门禁」—— 返回 `null` 表示未接。 */
function isGated(fnName, body) {
  if (GATE_MARKERS.some((re) => re.test(body))) return "自身门禁";
  if (!DAO_WRITE_FN_NAMES.has(fnName) && DAO_WRITE_CALLS.some((re) => re.test(body))) {
    return "经 DAO 收口";
  }
  return null;
}

// ── 自检（正负对照）────────────────────────────────────────

if (process.argv.includes("--selftest")) {
  const cases = [];
  const check = (name, actual, expected) => {
    const pass = JSON.stringify(actual) === JSON.stringify(expected);
    cases.push({ name, pass, actual, expected });
  };

  // ① 净化器：必须等长（行号才可信），且注释/字符串里的写入模式要被剥掉
  const src = [
    "fn a() {",
    "  // workflow_template::ActiveModel { 注释里的，不算写入点",
    '  let s = "workflow_template::Entity::insert 字符串里的";',
    "  let c = '\\n';",
    "  let ch = '中';",
    "  let lt: &'static str = \"x\"; // workflow_template::Column::Nodes",
    "  workflow_template::Entity::insert(am);",
    "}",
  ].join("\n");
  const clean = sanitize(src);
  const cl = clean.split("\n");
  check("sanitize 行数守恒", cl.length, src.split("\n").length);
  check("sanitize 长度守恒", clean.length, src.length);
  check("注释里的写入模式被剥", /ActiveModel\s*\{/.test(cl[1]), false);
  check("字符串里的写入模式被剥", /Entity::insert/.test(cl[2]), false);
  check("转义 char 不吞代码", /let c/.test(cl[3]), true);
  check("非 ASCII char 不吞代码", /let ch/.test(cl[4]), true);
  check("lifetime 不吞代码", /&'static str/.test(cl[5]), true);
  check("真代码行保留", /Entity::insert/.test(cl[6]), true);

  // ② 函数边界：Rust 允许嵌套 fn，命中行必须归给**包含它的**那个
  const nested = [
    "fn outer() {",
    "    let x = 1;",
    "    fn inner() {",
    "        let y = 2;",
    "    }",
    "    let z = 3;",
    "}",
  ];
  check("嵌套 fn：命中行归外层", enclosingFunction(nested, 5).name, "outer");
  check("嵌套 fn：命中行归内层", enclosingFunction(nested, 3).name, "inner");

  // ③ 判定：三条路径都要能区分（防「恒绿」与「恒红」两种退化）
  check("无门禁 ⇒ 未接", isGated("f", "let a = 1;"), null);
  check("enforce ⇒ 自身门禁", isGated("f", "enforce_port_axioms(&n, &e)?;"), "自身门禁");
  check(
    "enforce 的 JSON/ActiveModel 变体也认",
    isGated("f", "enforce_port_axioms_active_model(&t)?;"),
    "自身门禁"
  );
  check("warn ⇒ 自身门禁", isGated("f", 'warn_port_axioms_json("c", &n, &e);'), "自身门禁");
  check("is_port_axiom_clean ⇒ 自身门禁", isGated("f", "if is_port_axiom_clean(r) {}"), "自身门禁");
  check("经 DAO ⇒ 收口", isGated("f", "db_repo::insert_workflow_template(db, am)"), "经 DAO 收口");
  check("upsert 也算 DAO 收口", isGated("f", "upsert_workflow_template(db, am)"), "经 DAO 收口");
  check(
    "DAO 写函数自身不能自证",
    isGated("insert_workflow_template", "insert_workflow_template(db, am)"),
    null
  );

  // ④ 枚举模式：少了任何一条都会漏扫（本脚本第一版只有 A+B，漏掉裸 ActiveModel 与 Entity::insert）
  check("枚举模式数量 ≥ 5", WRITE_PATTERNS.length >= 5, true);
  check("模式 id 无重复", new Set(WRITE_PATTERNS.map((p) => p.id)).size, WRITE_PATTERNS.length);

  let failed = 0;
  for (const c of cases) {
    if (c.pass) console.log(`  ✓ ${c.name}`);
    else {
      failed++;
      console.log(`  ✗ ${c.name}\n      期望 ${JSON.stringify(c.expected)}，实得 ${JSON.stringify(c.actual)}`);
    }
  }
  console.log(`\n自检：${cases.length - failed} 通过 / ${failed} 失败`);
  process.exit(failed > 0 ? 1 : 0);
}

const files = SCAN_ROOTS.flatMap((r) => walk(r));
if (files.length === 0) {
  console.error("扫描失败：没有找到任何 .rs 文件（SCAN_ROOTS 可能写错了）");
  process.exit(2);
}

const rel = (p) => relative(REPO, p).split(sep).join("/");
const points = new Map();
const badLineCount = [];
const onlyInText = []; // 只在原文（注释/字符串）里命中、净化后消失的行 —— 供人工核对

for (const f of files) {
  const raw = readFileSync(f, "utf8");
  const clean = sanitize(raw);
  // 自证：净化必须严格等长（逐码元替换、换行原样），否则行号不可信、
  // 下面所有定位都是假的 —— 宁可整体报错也不输出带偏的行号。
  if (clean.split("\n").length !== raw.split("\n").length) {
    badLineCount.push(rel(f));
    continue;
  }
  const lines = clean.split("\n");
  const rawLines = raw.split("\n");
  const cleanHitLines = new Set();
  for (let i = 0; i < lines.length; i++) {
    for (const p of WRITE_PATTERNS) {
      if (!p.re.test(lines[i])) continue;
      if (p.needsFileContent && !p.needsFileContent.test(clean)) continue;
      // 版本快照表（workflow_template_version）不是主表，且**必须**允许存旧形态 ⇒ 排除
      const ctx = lines[i] + (lines[i - 1] || "");
      if (/workflow_template_version/.test(ctx)) continue;
      cleanHitLines.add(i);
      const fn = enclosingFunction(lines, i);
      const key = `${rel(f)}::${fn.name}`;
      if (!points.has(key)) {
        points.set(key, {
          file: rel(f),
          line: i + 1,
          fn: fn.name,
          patterns: new Set(),
          body: fn.text,
        });
      }
      points.get(key).patterns.add(p.id);
    }
  }

  // 交叉复现（判据 #7：审计工具自身会撒谎）：拿**未净化**的原文重扫一遍主模式，
  // 凡「原文命中、净化后不命中」的行都列出来。它只应该发生在注释/字符串里；
  // 若出现在普通代码行，说明净化器吞了代码（漏扫），必须人工介入。
  for (let i = 0; i < rawLines.length; i++) {
    for (const p of WRITE_PATTERNS) {
      if (!["A", "B", "C", "E"].includes(p.id)) continue;
      if (!p.re.test(rawLines[i])) continue;
      if (cleanHitLines.has(i)) continue;
      onlyInText.push({ file: rel(f), line: i + 1, text: rawLines[i].trim().slice(0, 100) });
    }
  }
}

// 模式 E（按列 update_many）只有与模式 B（真的写 nodes/edges 列）同函数时才算写入点。
for (const p of [...points.values()]) {
  for (const [need, dep] of Object.entries(PATTERN_REQUIRES)) {
    if (p.patterns.has(need) && !p.patterns.has(dep)) {
      p.patterns.delete(need);
      if (p.patterns.size === 0) points.delete(`${p.file}::${p.fn}`);
    }
  }
}
// 构造辅助自身不是落库点（其调用点由模式 C 覆盖）。
for (const [key, p] of [...points.entries()]) {
  if (CONSTRUCTOR_HELPERS.has(p.fn) && !p.patterns.has("C") && !p.patterns.has("E")) {
    points.delete(key);
  }
}

if (badLineCount.length > 0) {
  console.error(
    `扫描失败：${badLineCount.length} 个文件注释剥离后行数变化（行号不可信）：${badLineCount
      .slice(0, 5)
      .join(", ")}`
  );
  process.exit(2);
}

const list = [...points.values()].sort((a, b) =>
  a.file === b.file ? a.fn.localeCompare(b.fn) : a.file.localeCompare(b.file)
);

let unGated = 0;
console.log(`扫描 ${files.length} 个 .rs 文件，命中 ${list.length} 处「函数 × 写路径」：\n`);
for (const p of list) {
  const how = isGated(p.fn, p.body);
  const pats = [...p.patterns].sort().join("");
  if (how) {
    console.log(`  [已接] ${p.file}::${p.fn}  模式${pats}  ← ${how}`);
  } else {
    unGated++;
    console.log(`  [未接] ${p.file}:${p.line}  模式${pats}  fn=${p.fn}`);
  }
}

console.log(`\n汇总：已接 ${list.length - unGated}｜未接 ${unGated}（写入点函数合计 ${list.length}）`);

if (onlyInText.length > 0) {
  console.log(
    `\n提示：${onlyInText.length} 处「写入模式」只出现在注释或字符串里（已按空白处理）——` +
      `列出来供人工核对，若其中出现**普通代码行**则说明净化器吞了代码：`
  );
  for (const x of onlyInText) console.log(`  · ${x.file}:${x.line}  ${x.text}`);
}

if (unGated > 0) {
  console.error(
    `\nFAIL：${unGated} 个写路径没有接端口公理门禁。\n` +
      `  用户/LLM/导入路径 → enforce_port_axioms(&nodes, &edges)?\n` +
      `  我们写死的种子   → warn_port_axioms(context, &nodes, &edges)（记录不阻断）\n` +
      `  或者改为经 DAO 的 insert/upsert_workflow_template 落库（门禁在那一层）`
  );
  process.exit(1);
}
console.log("\nPASS：所有写 workflow_templates.nodes/edges 的函数都已接门禁。");
process.exit(0);
