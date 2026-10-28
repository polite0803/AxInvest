#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 荐股闭环「逐格状态」枚举覆盖门（2026-10-28）。
 *
 * ## 为什么需要它
 *
 * 直接起因是界面上出现了一串原始码：`disabled_negative_expectancy`。根因不是那条判据写错，
 * 而是**同一个状态词表被抄在四个面上，加状态时只改了其中两个**：
 *
 * | 面 | 位置 | 当时的真实形态 |
 * |---|---|---|
 * | 权威 | `recommender/reco_loop.rs` 的 `LoopCellStatus::as_str` | 已加 `disabled_negative_expectancy` |
 * | 前端类型 | `src/types/stock-analysis.ts` 的 `RecoLoopCell["status"]` 联合 | 已加 |
 * | 面板呈现 | `RecoLoopPanel.tsx` 的 `statusTag` switch | **漏** ⇒ 落到 `default:` 把后端枚举原文当文案渲染 |
 * | 矩阵呈现 | `RecoStrategyMatrix.tsx` 的闭环态分支 | **漏得更坏** ⇒ 新状态静默走进 else，显示成「闭环已校准 权重 0.00」 |
 *
 * `tsc` / `cargo check` / vitest 全绿也照样存在：面板 switch 有 `default` 兜底，矩阵的 else
 * 分支对任何未知状态都成立 —— **兜底把「没跟上」洗成了「看着正常」**。文案面同理：少一个键
 * 只会显示原始码或空串，没有一条既有门会因此红。
 *
 * ## 判据
 *
 *   R0  四个抽取面任一为空 ⇒ exit 2（扫到 0 条的门等于没有门，同 `check-reco-horizon-parity.mjs`）。
 *   R1  后端 `as_str` 码集 == TS `RecoLoopCell["status"]` 联合（双向）。
 *   R2  后端码集 == 面板显式处理的状态集（`case "x"` 或 `status === / !== "x"`，双向；多余项
 *        会静默死掉，同样算红）。上游过滤掉的态不强求再写 case —— 那是永不命中的死分支。
 *   R3  后端码集 == 矩阵 `LOOP_CELL_VIEW` 穷举表键集（双向）。
 *   R4  面板兜底分支在位，且渲染**原始码 + statusUnknown 句**（不许删成 `undefined`，
 *        也不许改成谎报某个已知状态）。
 *   R5  四个状态文案键（`recoLoop.statusDisabled` / `.statusUnknown` / `.disabledTip` /
 *        `backtest.loopDisabled`）在 11 种语言里都存在且非空 —— `check-i18n-key-parity.mjs`
 *        只断「11 份集合一致」，全缺也照样绿，所以存在性单独钉一条。
 *
 * 用法：
 *   node scripts/check-reco-loop-status-coverage.mjs            # 比对（有违规 exit 1）
 *   node scripts/check-reco-loop-status-coverage.mjs --selftest # 负控：合成源码喂判据，逐条必须红
 */

import fs from "node:fs";
import path from "node:path";
import process from "node:process";

import { matchBraceEnd } from "./lib/rust-source-scan.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const RECO_LOOP_RS = "src-tauri/crates/analysis-engine/src/recommender/reco_loop.rs";
const TYPES_TS = "src/types/stock-analysis.ts";
const PANEL_TSX = "src/components/stock-analysis/RecoLoopPanel.tsx";
const MATRIX_TSX = "src/components/stock-analysis/RecoStrategyMatrix.tsx";
const LOCALES = ["ar", "de", "en-US", "es", "fr", "hi", "ja", "ko", "ru", "zh-CN", "zh-TW"];
const REQUIRED_KEYS = [
  ["stockAnalysis", "recoLoop", "statusDisabled"],
  ["stockAnalysis", "recoLoop", "statusUnknown"],
  ["stockAnalysis", "recoLoop", "disabledTip"],
  ["stockAnalysis", "backtest", "loopDisabled"],
];

const read = (rel) => fs.readFileSync(path.join(ROOT, rel), "utf8");
const literals = (text, re) => [...text.matchAll(re)].map((m) => m[1]);
const lineOf = (src, needle) => {
  const i = src.indexOf(needle);
  return i < 0 ? -1 : src.slice(0, i).split("\n").length;
};

/** 从「声明行」起取到它那个**平衡闭合**的整段（声明行自带 `{` 的形态）。 */
function regionOfBalanced(src, declPredicate) {
  const lines = src.split("\n");
  const start = lines.findIndex(declPredicate);
  if (start < 0) { return null; }
  return lines.slice(start, matchBraceEnd(lines, start) + 1).join("\n");
}

/** 从「声明行」起向后找到赋值的那个 `{`，再取到它的闭合 `}`。
 *  不能直接从声明行走 `matchBraceEnd`：TS 的 `Record<... | { ... } | null> = {` 里
 *  类型标注先出现一对花括号，会在类型闭合处就返回，抽到的是类型而不是值。 */
function regionOfAssignedValue(src, declPredicate) {
  const lines = src.split("\n");
  const decl = lines.findIndex(declPredicate);
  if (decl < 0) { return null; }
  let open = decl;
  while (open < lines.length && !/=\s*\{$/.test(lines[open])) { open += 1; }
  if (open >= lines.length) { return null; }
  return lines.slice(open, matchBraceEnd(lines, open) + 1).join("\n");
}

/** R1 用：后端 `as_str` 的取值（权威词表）。 */
function backendCodes(rsSrc) {
  const impl = regionOfBalanced(rsSrc, (l) => /^impl LoopCellStatus \{/.test(l));
  if (impl === null) { return null; }
  const at = impl.indexOf("fn as_str");
  if (at < 0) { return null; }
  return literals(impl.slice(at), /=>\s*"([a-z0-9_]+)"/g);
}

/** R1 用：前端联合类型里的码。 */
function unionCodes(tsSrc) {
  const iface = regionOfBalanced(tsSrc, (l) => l.startsWith("export interface RecoLoopCell {"));
  if (iface === null) { return null; }
  const block = iface.match(/\n\s*status:\s*([\s\S]*?);/);
  return block === null ? null : literals(block[1], /"([a-z0-9_]+)"/g);
}

/** R2 用：面板里显式处理过的码。
 *  两种合法写法都算覆盖：switch 的 `case "x"`，与上游的 `status === / !== "x"` 过滤
 *  （`not_in_matrix` 就是后者 —— 它在进表格前就被滤掉，给它写 case 反而是永不命中的死分支）。 */
function panelCodes(paneSrc) {
  const fn = regionOfBalanced(paneSrc, (l) => l.includes("const statusTag = "));
  if (fn === null) { return null; }
  return [...new Set([
    ...literals(fn, /\bcase "([a-z0-9_]+)":/g),
    ...literals(paneSrc, /\bstatus\s*[!=]==\s*"([a-z0-9_]+)"/g),
  ])];
}

/** R3 用：矩阵穷举表的键（值对象里的 2 缩进键）。 */
function matrixKeys(matrixSrc) {
  const table = regionOfAssignedValue(matrixSrc, (l) => l.includes("const LOOP_CELL_VIEW: Record<"));
  return table === null ? null : literals(table, /^ {2}([a-z][a-z0-9_]*):/gm);
}

/** 码在文件里的行号：前端用引号字面量，穷举表的键是裸标识符。 */
const where = (src, code) => {
  const quoted = lineOf(src, `"${code}"`);
  return quoted > 0 ? quoted : lineOf(src, code);
};

function diffSets(authority, other, authoritySrc, otherSrc, otherLabel) {
  const violations = [];
  for (const code of authority) {
    if (!other.includes(code)) {
      violations.push(
        `缺 ${code} —— ${otherLabel}:${where(otherSrc, code)} 没这一臂，新状态会静默走进兜底/else`
        + `（权威在 ${RECO_LOOP_RS}:${lineOf(authoritySrc, `"${code}"`)}）`,
      );
    }
  }
  for (const code of other) {
    if (!authority.includes(code)) {
      violations.push(
        `多出 ${code} —— ${otherLabel}:${where(otherSrc, code)}`
        + ` 有本门认不出的陈旧项（会变成永不命中的死分支）`,
      );
    }
  }
  return violations;
}

/** R4：兜底分支必须存在且如实露出原始码。 */
function checkUnknownFallback(paneSrc) {
  const violations = [];
  if (!/^\s*default:$/m.test(paneSrc)) {
    violations.push(`${PANEL_TSX} 的 statusTag 没有 default 分支：未知状态会渲染成 undefined`);
  }
  if (!paneSrc.includes('data-testid="reco-loop-status-unknown"')) {
    violations.push(`${PANEL_TSX} 缺未知状态分支的 testid（兜底被删，或被改成谎报某个已知状态）`);
  }
  if (!paneSrc.includes("stockAnalysis.recoLoop.statusUnknown")) {
    violations.push(`${PANEL_TSX} 的兜底没走 statusUnknown 文案`);
  }
  if (!/code:\s*cell\.status/.test(paneSrc)) {
    violations.push(`${PANEL_TSX} 的兜底没把原始码带进文案（界面丢了信息，只能靠猜）`);
  }
  return violations;
}

/** R5：新键必须在 11 语言里都在。 */
function checkLocaleKeys(loadDoc) {
  const violations = [];
  for (const loc of LOCALES) {
    const rel = `src/i18n/locales/${loc}.json`;
    let data;
    try {
      data = loadDoc(loc);
    } catch (e) {
      violations.push(`${rel} 解析失败：${e.message}`);
      continue;
    }
    for (const keys of REQUIRED_KEYS) {
      let node = data;
      for (const k of keys) { node = node?.[k]; }
      if (typeof node !== "string" || node.trim() === "") {
        violations.push(`${rel} 缺 ${keys.join(".")}`);
      }
    }
  }
  return violations;
}

function runChecks(sources, loadDoc = (loc) => JSON.parse(read(`src/i18n/locales/${loc}.json`))) {
  const rsSrc = sources[RECO_LOOP_RS];
  const tsSrc = sources[TYPES_TS];
  const paneSrc = sources[PANEL_TSX];
  const matrixSrc = sources[MATRIX_TSX];

  const authority = backendCodes(rsSrc);
  const union = unionCodes(tsSrc);
  const cases = panelCodes(paneSrc);
  const tableKeys = matrixKeys(matrixSrc);

  // R0 —— 任何一面抽到空 = 本门在被检对象上没电，直接 fatal 而不是「绿」
  const empty = [
    [RECO_LOOP_RS, authority],
    [TYPES_TS, union],
    [PANEL_TSX, cases],
    [MATRIX_TSX, tableKeys],
  ].filter(([, v]) => v === null || v.length === 0).map(([f]) => f);
  if (empty.length > 0) {
    return { fatal: `抽取到空集合，扫描面失效：${empty.join(", ")}`, rules: [], authority: [] };
  }

  const rules = [
    ["R1 后端 as_str 码集 == 前端 RecoLoopCell 状态联合", diffSets(authority, union, rsSrc, tsSrc, TYPES_TS)],
    [
      "R2 后端码集 == 面板显式处理的状态（case 或 status 字面量比较）",
      diffSets(authority, cases, rsSrc, paneSrc, PANEL_TSX),
    ],
    [
      "R3 后端码集 == 矩阵 LOOP_CELL_VIEW 穷举表键集",
      diffSets(authority, tableKeys, rsSrc, matrixSrc, MATRIX_TSX),
    ],
    ["R4 面板兜底在位且如实露原始码", checkUnknownFallback(paneSrc)],
    ["R5 四个状态文案键在 11 语言齐备", checkLocaleKeys(loadDoc)],
  ];
  return { fatal: null, rules, authority };
}

function loadSources() {
  return {
    [RECO_LOOP_RS]: read(RECO_LOOP_RS),
    [TYPES_TS]: read(TYPES_TS),
    [PANEL_TSX]: read(PANEL_TSX),
    [MATRIX_TSX]: read(MATRIX_TSX),
  };
}

function selftest() {
  const sources = loadSources();
  const baseline = runChecks(sources);
  if (baseline.fatal !== null) {
    console.error(`基线就没电：${baseline.fatal}`);
    process.exit(2);
  }
  const redRules = (mutated, loadDoc) =>
    runChecks(mutated, loadDoc).rules.filter(([, v]) => v.length > 0).map(([r]) => r.slice(0, 2));

  const cases = [
    {
      name: "后端新增状态、三个消费面都没跟上 ⇒ R1+R2+R3 同红",
      mutate: (s) => {
        s[RECO_LOOP_RS] = s[RECO_LOOP_RS].replace(
          'LoopCellStatus::DisabledNegativeExpectancy => "disabled_negative_expectancy",',
          'LoopCellStatus::DisabledNegativeExpectancy => "disabled_negative_expectancy",\n            LoopCellStatus::BrandNew => "brand_new_status",',
        );
      },
      expect: ["R1", "R2", "R3"],
    },
    {
      name: "面板把一个 case 打错名 ⇒ 只 R2 红（缺一个 + 多一个死分支）",
      mutate: (s) => {
        s[PANEL_TSX] = s[PANEL_TSX].replace(
          'case "disabled_negative_expectancy":',
          'case "retired_status_name":',
        );
      },
      expect: ["R2"],
    },
    {
      name: "面板既不给 not_in_matrix 写 case、又删掉上游过滤 ⇒ 只 R2 红",
      mutate: (s) => {
        s[PANEL_TSX] = s[PANEL_TSX].replace(
          'c.status !== "not_in_matrix"',
          'c.status !== "other_placeholder"',
        );
      },
      expect: ["R2"],
    },
    {
      name: "前端联合留了后端已撤的码 ⇒ 只 R1 红",
      mutate: (s) => {
        s[TYPES_TS] = s[TYPES_TS].replace(
          '    | "disabled_negative_expectancy";',
          '    | "disabled_negative_expectancy"\n    | "retired_status_name";',
        );
      },
      expect: ["R1"],
    },
    {
      name: "矩阵穷举表少一臂 ⇒ 只 R3 红",
      mutate: (s) => {
        s[MATRIX_TSX] = s[MATRIX_TSX].replace(/^ {2}ic_unmeasurable: \{/m, "  ic_unmeasurable_gone: {");
      },
      expect: ["R3"],
    },
    {
      name: "注释掉面板兜底 ⇒ 只 R4 红",
      mutate: (s) => {
        s[PANEL_TSX] = s[PANEL_TSX].replace(/^(\s*)default:$/m, "$1// default:");
      },
      expect: ["R4"],
    },
    {
      name: "兜底不再带原始码 ⇒ 只 R4 红",
      mutate: (s) => {
        s[PANEL_TSX] = s[PANEL_TSX].replace("code: cell.status", 'code: "fixed"');
      },
      expect: ["R4"],
    },
    {
      name: "某个 locale 缺 statusDisabled ⇒ 只 R5 红",
      mutate: () => {},
      loadDoc: (loc) => {
        const doc = JSON.parse(read(`src/i18n/locales/${loc}.json`));
        if (loc === "de") { delete doc.stockAnalysis.recoLoop.statusDisabled; }
        return doc;
      },
      expect: ["R5"],
    },
    {
      name: "非破坏样本必须全绿（否则上面这些红没有分辨力）",
      mutate: () => {},
      expect: [],
    },
  ];

  let bad = 0;
  for (const c of cases) {
    const s = loadSources();
    c.mutate(s);
    // mutate 只动字符串面；R5 读盘，所以给它一份「磁盘等价」的 loader，除显式声明外
    const loadDoc = c.loadDoc ?? ((loc) => JSON.parse(read(`src/i18n/locales/${loc}.json`)));
    const hit = redRules(s, loadDoc);
    const ok = hit.length === c.expect.length && c.expect.every((e) => hit.includes(e));
    console.log(`${ok ? "✓" : "✗"} ${c.name}${ok ? "" : ` → 实际红：[${hit.join(",")}]`}`);
    if (!ok) { bad += 1; }
  }

  // R0 自身：喂一份「没有 impl LoopCellStatus」的后端源码 ⇒ 必须 fatal，而不是绿
  const dead = loadSources();
  dead[RECO_LOOP_RS] = "// 没有 LoopCellStatus 的桩\n";
  const fatal = runChecks(dead).fatal !== null;
  console.log(`${fatal ? "✓" : "✗"} 后端源里抽不到状态词表 ⇒ 门判「没电」而不是绿`);
  if (!fatal) { bad += 1; }

  process.exit(bad === 0 ? 0 : 1);
}

function main() {
  const { fatal, rules, authority } = runChecks(loadSources());
  if (fatal !== null) {
    console.error(`FATAL ${fatal}`);
    process.exit(2);
  }
  let bad = 0;
  for (const [rule, violations] of rules) {
    if (violations.length === 0) {
      console.log(`  ok  ${rule}`);
      continue;
    }
    bad += 1;
    console.log(`FAIL  ${rule}`);
    for (const v of violations) { console.log(`        ${v}`); }
  }
  console.log(`\n权威词表 ${authority.length} 个状态（${RECO_LOOP_RS}）：${authority.join(", ")}`);
  if (bad > 0) {
    console.log("\n后端加状态时四个面必须同批改：前端联合 / 面板 case / 矩阵穷举表 / 11 语言文案。");
    process.exit(1);
  }
}

if (process.argv.includes("--selftest")) { selftest(); } else { main(); }
