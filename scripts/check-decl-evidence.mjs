// 校验 `crates/harness/src/knowledge_graph.rs` 里三张声明表的 evidence（`文件:行` 出处）
// 是否仍然指得对 —— 防「引用腐烂」（改了源码、行号漂移、出处指向别的东西）。
//
// ⚠ 这是**人工复核辅助**，不是门禁：它只证明「行号可定位 + 该行非空」，
//   「这行内容是否真属于这条声明」仍需人判 —— 故输出按 id 配对打印，不再只丢一行行号。
//
// 设计依据（判据 #7 审计脚本自身会撒谎）：
//   - 三类证据必须分开：LOCATED（可定位）/ NONLOC（本来就没有文件:行，如 DB 分布、CSV 行数）
//     / BROKEN（文件不存在 或 行号越界）。只有 BROKEN 才是失败。
//   - 旧版把 NONLOC 也当失败 ⇒ 恒定假红（6 条 DB/CSV 证据永远"未能定位"）。
//   - LOCATED == 0 ⇒ 非 0 退出（否则解析器坏了也会"全绿"）。
//   - --selftest 用合成样本做正负对照，证明分类器真的会红。
//
// 用法：
//   node scripts/check-decl-evidence.mjs
//   node scripts/check-decl-evidence.mjs --selftest
//   node scripts/check-decl-evidence.mjs --ci        # CI 模式：容忍「内容存疑」，只拦真腐烂
//
// 退出码：0 全好 ｜ 1 引用腐烂（真失败）｜ 2 脚本自身失效（源文件不在 / 一条声明都解析不到）
//        ｜ 3 只有「内容存疑」（需人判，**非失败** —— 见下）
//
// 为什么 --ci 只拦 1 不拦 3：结构腐烂（文件没了 / 行号越界 / 指到空行）是**客观错**，
// 修法唯一；而「内容与 id 不同源」是**软判据**，它的假阳性来自「同一件事换了措辞」
// （如 causes 用常量名写入），会随无关编辑抖动。硬拦它 ⇒ 红灯要靠改判据才灭，
// 结果必然是被绕过。故 CI 里它只打印、不失败。
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, "..");
const SRC = path.join(ROOT, "src-tauri/crates/harness/src/knowledge_graph.rs");
const REPO = path.join(ROOT, "src-tauri");

const CI_MODE = process.argv.includes("--ci");
// ⚠ 这张表**必须与 `harness/src/knowledge_graph.rs` 里实际存在的声明表一一对应**：
//   漏一个类型 ⇒ 该表的 evidence 全部**静默不受检查**（不是"检查通过"，是"从没检查过"）。
//   实测踩过：2026-09-14 新增 `EdgeTypeDecl`（6 条声明）后本表没跟，新表零覆盖，
//   而脚本照样报"腐烂 0" —— 判据 #7/#130「审计工具自身会撒谎」的典型形态。
//
//   **刻意只维护一份数据**（struct 名 + 常量名成对）：分列两张清单（一张类型名、
//   一张常量名）必然漂移 —— 改了一处忘另一处，就又回到上面那个坑里。
const DECL_TABLES = [
  { type: "RelationDecl", konst: "RELATION_DECLS" },
  { type: "EntityTypeDecl", konst: "ENTITY_TYPE_DECLS" },
  { type: "DataDrivenColumnDecl", konst: "DATA_DRIVEN_COLUMN_DECLS" },
  { type: "EdgeTypeDecl", konst: "EDGE_TYPE_DECLS" },
];
const DECL_TYPES = DECL_TABLES.map((t) => t.type);
const START_RE = new RegExp(`(?:^\\s*|=\\s*)(${DECL_TYPES.join("|")})\\s*\\{\\s*$`);
const END_RE = /^\s*\}\s*,?\s*$/;
const FILE_EXT = "rs|sh|mjs|ts|tsx|json";
// ⚠ 必须扫**一段里的全部**引用，不能只看第一个：实测 `in_industry` 的 evidence 写成
//   「src/commands/knowledge.rs:1782（另有文档示例 harness/src/knowledge_graph.rs:195）」，
//   只取首个匹配就会漏掉后半段那条腐烂引用（判据 #7：审计脚本自身会撒谎）。
// 裸 `:行号`（同段内承接前一个文件名）也要认，否则同样漏扫。
const REF_G = new RegExp(`(?:([A-Za-z0-9_./-]+\\.(?:${FILE_EXT})))?:(\\d+)`, "g");

/** 从源码里抽出所有声明块（行扫描：字段各占一行，块以 `}` 收尾） */
function parseDecls(text) {
  const lines = text.split(/\r?\n/);
  const out = [];
  let cur = null;
  for (const L of lines) {
    if (cur) {
      cur.lines.push(L);
      if (END_RE.test(L)) {
        const body = cur.lines.join("\n");
        const v = (k) => body.match(new RegExp(`${k}:\\s*"((?:[^"\\\\]|\\\\.)*)"`))?.[1];
        out.push({
          type: cur.type,
          line: cur.line,
          id: v("id") ?? v("column") ?? "(无 id)",
          evidence: v("evidence") ?? "",
          observed: /observed:\s*true/.test(body),
        });
        cur = null;
      }
      continue;
    }
    if (START_RE.test(L)) cur = { type: L.trim().replace(/^.*=\s*/, "").replace(/\s*\{$/, ""), line: out.length, lines: [L] };
  }
  return out;
}

/** 单个 evidence 串 → 逐条判定（纯函数，便于 --selftest 复用） */
function classify(evidence, readFile) {
  const res = [];
  // 文件名跨「段」承接：`a.rs:2 ｜ :99` 里的 `:99` 仍指 a.rs
  let lastFile = null;
  for (const raw of evidence.split(/[｜|]/)) {
    const part = raw.trim();
    if (!part) continue;
    // 扫该段里的**全部**引用（含裸 `:行号` 承接前一个文件名）
    let hit = false;
    REF_G.lastIndex = 0;
    for (const m of part.matchAll(REF_G)) {
      const rel = m[1] ?? lastFile;
      const line = Number(m[2]);
      if (!rel) continue; // 全串没有任何文件名 ⇒ 交给下面的 NONLOC 分支
      lastFile = rel;
      hit = true;
      // 行尾括注（如「排除」「type 列，20872 行」）不是引用的一部分，剥掉后再匹配
      const content = readFile(rel);
      const ref = `${rel}:${line}`;
      if (content === null) {
        res.push({ part, ref, kind: "BROKEN", note: "文件不存在" });
        continue;
      }
      const arr = content.split(/\r?\n/);
      if (line < 1 || line > arr.length) {
        res.push({ part, ref, kind: "BROKEN", note: `行号越界（文件仅 ${arr.length} 行）` });
        continue;
      }
      const text = arr[line - 1].trim();
      if (text === "") {
        res.push({ part, ref, kind: "BROKEN", note: "该行是空行 ⇒ 行号已漂移" });
        continue;
      }
      res.push({ part, ref, kind: "LOCATED", content: text });
    }
    if (!hit) {
      res.push({ part, kind: "NONLOC", note: "无 文件:行（DB 分布 / 数据文件统计 类证据）" });
    }
  }
  return res;
}

// ── 软判据：被引用的那一行是否**与 id 同源** ──────────────────────────
//
// 结构校验（文件在不在 / 行号越不越界 / 是不是空行）抓不到「行号没越界但指向
// 完全无关的一行」这种腐烂 —— 实测 `uses` 曾指向一句 `assert_eq!(…, "mentions")`。
// 故补一条形态判据：被引用的行里必须出现 id（大小写不敏感）。
//
// ⚠ 白名单 = 人工判读后确认「对，但不同源」的例外；新增条目必须有理由，不能图省事。
const SOFT_ALLOW = new Map([
  ["causes", "写入端用常量 CAUSAL_RELATION_TYPE（同义不同名），行内不含 causes"],
  ["knowledge_relations.relation_type", "id 是 DB 列名，出处在讲 CSV 的 rtype 列"],
  [
    "reference",
    "写入端调构造器 `GraphEdge::relation(…)`（字面量 `\"reference\"` 内置在 `graph_dtos.rs:85` 的构造器体内）；" +
      "这是 2026-09-14 拆双字段后的形态 —— 好处正是「reference」只有一处字面量",
  ],
]);
const sameSource = (id, content) => content.toLowerCase().includes(id.toLowerCase());

// ── selftest（正负对照）──────────────────────────────────────────────

if (process.argv.includes("--selftest")) {
  const fake = (p) => (p === "ok.rs" ? "let x = 1;\nlet y = 2;\n\n" : null);
  /** [evidence, 期望的 kind 序列] —— 序列而非单值：多引用段必须逐条都在 */
  const cases = [
    ["ok.rs:2", ["LOCATED"]],
    ["ok.rs:99", ["BROKEN"]],
    ["ok.rs:3", ["BROKEN"]],
    ["no_such.rs:1", ["BROKEN"]],
    ["DB 分布（实测）", ["NONLOC"]],
    ["nodes.csv（type 列，20872 行）", ["NONLOC"]],
    // 回归：一段里的第二个引用（括注形态）过去被静默漏扫
    ["ok.rs:2（另有 ok.rs:99）", ["LOCATED", "BROKEN"]],
    // 回归：裸 `:行号` 应承接前一个文件名
    ["ok.rs:2 ｜ :99", ["LOCATED", "BROKEN"]],
  ];
  let bad = 0;
  for (const [ev, want] of cases) {
    const got = classify(ev, fake).map((r) => r.kind);
    const ok = JSON.stringify(got) === JSON.stringify(want);
    if (!ok) bad++;
    console.log(`  ${ok ? "✔" : "✖"} ${ev.padEnd(30)} want=${want.join("+")} got=${got.join("+")}`);
  }
  console.log(
    bad === 0
      ? `✔ selftest 通过（${cases.length} 例 / 4 类样本上的正负对照，含 2 例漏扫回归）`
      : `✖ selftest 失败 ${bad} 例`,
  );

  // ── 覆盖率守卫的负样本对照：必须能证明它**会红** ──────────────────
  // 只证明「现在不红」是不够的 —— 一条永远不会红的守卫等于没有守卫。
  const covOk = coverageGaps(
    "pub const A_DECLS: &[ADecl] = &[];",
    [{ type: "ADecl" }],
    [{ type: "ADecl", konst: "A_DECLS" }],
  );
  const covMissing = coverageGaps(
    "pub const A_DECLS: &[ADecl] = &[];\npub const B_DECLS: &[BDecl] = &[];",
    [{ type: "ADecl" }],
    [{ type: "ADecl", konst: "A_DECLS" }],
  );
  const covEmpty = coverageGaps(
    "pub const A_DECLS: &[ADecl] = &[];",
    [],
    [{ type: "ADecl", konst: "A_DECLS" }],
  );
  const covCases = [
    ["登记齐全", covOk, 0, 0],
    ["源码多一张未登记的表", covMissing, 1, 0],
    ["登记了但解析不到块", covEmpty, 0, 1],
  ];
  let covBad = 0;
  for (const [name, got, wantUnreg, wantEmpty] of covCases) {
    const ok = got.unregistered.length === wantUnreg && got.emptyTables.length === wantEmpty;
    if (!ok) covBad++;
    console.log(
      `  ${ok ? "✔" : "✖"} 覆盖率守卫·${name.padEnd(20)} ` +
        `want=(${wantUnreg},${wantEmpty}) got=(${got.unregistered.length},${got.emptyTables.length})`,
    );
  }
  if (covBad > 0) console.log(`✖ 覆盖率守卫 selftest 失败 ${covBad} 例（守卫不会红 = 等于没有）`);
  else console.log("✔ 覆盖率守卫正负对照通过（含 2 例必须变红的负样本）");
  process.exit(bad === 0 && covBad === 0 ? 0 : 1);
}

// ── 主流程 ───────────────────────────────────────────────────────────

if (!fs.existsSync(SRC)) {
  console.error(`✖ 源文件不存在：${SRC}`);
  process.exit(2);
}
const SRC_TEXT = fs.readFileSync(SRC, "utf8");
const decls = parseDecls(SRC_TEXT);
if (decls.length === 0) {
  console.error("✖ 未解析到任何声明块 —— 解析器已失效（不是「声明为空」）");
  process.exit(2);
}

// ── 覆盖率自证（防「新表静默不受检查」）──────────────────────────────
// 判据：源码里每个 `pub const <X>_DECLS` 都必须在 `DECL_TABLES` 里有一个类型与之配对；
// 反过来，`DECL_TABLES` 里登记的类型也必须真的解析到了块。
// 两头都查 ⇒ 漏登记 / 登记了但解析不到，都会当场红，而不是静默放过。
//
// 抽成纯函数是为了能在 --selftest 里做**负样本对照** —— 一条从没红过的守卫
// 本身也是不可信的（判据 #7：审计脚本会撒谎）。
function coverageGaps(srcText, decls, tables) {
  const foundConsts = [...srcText.matchAll(/pub const ([A-Z_]*_DECLS)\s*:/g)].map((m) => m[1]);
  const known = new Set(tables.map((t) => t.konst));
  const parsedTypes = new Set(decls.map((d) => d.type));
  return {
    unregistered: foundConsts.filter((c) => !known.has(c)),
    emptyTables: tables.filter((t) => !parsedTypes.has(t.type)).map((t) => t.type),
  };
}
const { unregistered, emptyTables } = coverageGaps(SRC_TEXT, decls, DECL_TABLES);
if (unregistered.length > 0 || emptyTables.length > 0) {
  console.error("✖ 声明表覆盖率自证失败（不是「没问题」，是「本脚本没在看」）：");
  if (unregistered.length) {
    console.error(`  · 源码里有 ${unregistered.length} 张表未登记进 DECL_TABLES：${unregistered.join(", ")}`);
  }
  if (emptyTables.length) {
    console.error(`  · DECL_TABLES 登记了但一条都没解析到（struct 名写错 / 解析器失效）：${emptyTables.join(", ")}`);
  }
  console.error("  ⇒ 把这些表补进 DECL_TABLES（`{ type, konst }` 成对）后重跑。");
  process.exit(2);
}

const cache = new Map();
const readFile = (rel) => {
  if (!cache.has(rel)) {
    const cands = [path.join(REPO, rel), path.join(REPO, "crates", rel), path.join(ROOT, rel)];
    const hit = cands.find((c) => fs.existsSync(c));
    cache.set(rel, hit ? fs.readFileSync(hit, "utf8") : null);
  }
  return cache.get(rel);
};

let located = 0;
let soft = 0;
const broken = [];
const rows = [];
for (const d of decls) {
  const parts = classify(d.evidence, readFile);
  located += parts.filter((p) => p.kind === "LOCATED").length;
  for (const p of parts) {
    if (p.kind !== "LOCATED") continue;
    p.soft = !sameSource(d.id, p.content);
    p.allow = SOFT_ALLOW.get(d.id);
    if (p.soft && !p.allow) soft++;
  }
  broken.push(...parts.filter((p) => p.kind === "BROKEN").map((p) => ({ ...p, id: d.id })));
  rows.push({ d, parts });
}

console.log(`── ${path.relative(ROOT, SRC)} ──`);
console.log(
  `  声明块 ${decls.length}｜证据条目 ${rows.reduce((a, r) => a + r.parts.length, 0)}｜` +
    `可定位 ${located}｜腐烂 ${broken.length}｜内容存疑 ${soft}`,
);
console.log();
for (const { d, parts } of rows) {
  console.log(`[${d.type.replace("Decl", "")}] ${d.id}${d.observed ? "" : "  ⚠observed=false"}`);
  for (const p of parts) {
    if (p.kind === "LOCATED") {
      const mark = p.allow ? "·" : p.soft ? "~" : "✔";
      console.log(`    ${mark} ${p.ref}\n        ${p.content.slice(0, 140)}`);
      if (p.soft && p.allow) console.log(`        （白名单：${p.allow}）`);
    } else if (p.kind === "NONLOC") console.log(`    · ${p.part}   （${p.note}）`);
    else console.log(`    ✖ ${p.ref}   ${p.note}`);
  }
}

// 退出码分级：0 全好｜1 引用腐烂（真失败）｜3 只有「内容存疑」（需人判，非失败）
if (broken.length > 0 || located === 0) {
  console.error(`\n✖ 判据不通过：腐烂 ${broken.length} 条｜可定位 ${located} 条`);
  if (located === 0) {
    console.error("  （可定位 0 条 ⇒ 先怀疑解析器失效，而不是「声明表本来就是空的」）");
  }
  process.exit(1);
}
if (soft > 0) {
  console.error(`\n~ 结构无腐烂，但 ${soft} 条引用的**内容与 id 不同源**（见上行 ~ 标记）⇒ 需人判是否已腐烂`);
  if (CI_MODE) {
    // 只打印不失败：CI 里输出到 stderr 不会让步骤转红（GitHub Actions 只看退出码）。
    console.error("  （--ci：本项为软判据，按设计容忍；要人工复核请本地跑不带 --ci 的版本）");
    process.exit(0);
  }
  process.exit(3);
}
console.log(`\n✔ ${located} 条引用行号可定位，且内容与所属 id 同源（白名单 ${SOFT_ALLOW.size} 条例外已登记）`);
