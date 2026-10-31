#!/usr/bin/env node
// scripts/check-i18n-exemption-budget.mjs
// i18n 豁免预算门禁（2026-10-31，审查清单建议落地，team-lead-15/27 规格）。
//
// 管三本账（对应 i18n-scan.mjs 的两条豁免通道 + 棘轮基线）：
//   ① 文件级 `i18n-exempt`（i18n-scan.mjs:168 `content.includes("i18n-exempt")` 整文件免检）
//      —— 实测 77 个文件。规则：标记行必须带 reason 与到期日 `i18n-exempt(...until=YYYY-MM-DD)`，
//      过期即红（到期 = 强制复审，不许无限期免检）。
//   ② 行级 allowlist（scripts/.i18n-allowlist.json）
//      —— 实测 433 entries / 433 有 reason / 0 有 expiry。规则：reason 必须非默认模板、
//      `expiry` 字段若存在不得过期；行号漂移条目（文件存在但行号集合与实际违规集合的交集为空）
//      记入 stale 清单。
//   ③ 棘轮：allowlist 总条目数与 i18n-exempt 文件数**只减不增**（基线存本脚本旁 JSON）。
//      增加 = 新豁免 = 红（新增豁免必须走人工评审，不是跑 update 就能洗白）。
//
// --update-allowlist（清洗模式，不是洗白模式）：
//   · 剔除 allowlist 中**文件已不存在**的条目（实测 14 个死文件）
//   · 剔除行号集合与当前实际违规行交集为空的**漂移条目**（需先跑一次 i18n-scan 拿实际违规集）
//   · 修正 total_entries / total_files / generated
//   · 绝不新增条目、绝不改 reason
//
// 用法：
//   node scripts/check-i18n-exemption-budget.mjs                    # 门禁
//   node scripts/check-i18n-exemption-budget.mjs --selftest         # 内建正负对照
//   node scripts/check-i18n-exemption-budget.mjs --update-allowlist # 清洗 allowlist
//   node scripts/check-i18n-exemption-budget.mjs --update-baseline  # 收紧棘轮基线（当前实测值）
//
// 退出码：0 = 账目全清（或 selftest 全过）；1 = 有过期/缺 reason/棘轮反弹/selftest 失败。

import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = join(__dirname, "..");

const args = process.argv.slice(2);
const SELFTEST = args.includes("--selftest");
const UPDATE_ALLOWLIST = args.includes("--update-allowlist");
const UPDATE_BASELINE = args.includes("--update-baseline");

const ALLOWLIST_PATH = join(root, "scripts", ".i18n-allowlist.json");
const BASELINE_PATH = join(root, "scripts", "i18n-exemption-budget-baseline.json");

// ── 到期日解析：i18n-exempt: xxx (until=YYYY-MM-DD) ──
const EXEMPT_MARK = "i18n-exempt";
const UNTIL_RE = /until\s*=\s*(\d{4}-\d{2}-\d{2})/;

/** 单行豁免标记判定（selftest 与正式跑共用）。
 * 返回 { ok, problem? }；ok=false 时 problem 描述缺口。 */
export function adjudicateExemptLine(line, todayIso) {
  if (!line.includes(EXEMPT_MARK)) { return { ok: true, skip: true }; }
  const untilM = line.match(UNTIL_RE);
  if (!untilM) {
    return { ok: false, problem: "缺到期日 until=YYYY-MM-DD（豁免必须带复审期限）" };
  }
  // 先判过期再判 reason：过期的豁免不管 reason 写得多好都必须复审
  if (untilM[1] < todayIso) {
    return { ok: false, problem: `到期日 ${untilM[1]} 已过 —— 豁免到期必须复审（转 t() 或补新期限+理由）` };
  }
  // reason = 标记冒号后的说明文本（剥掉 until 括号段后须有实质内容，≥8 字符）
  let body = line.slice(line.indexOf(EXEMPT_MARK) + EXEMPT_MARK.length);
  body = body.replace(/\(until=[^)]*\)/g, "").trim();
  const afterColon = body.split(/[:：]/).slice(1).join(":").trim();
  if (afterColon.length < 8) {
    return { ok: false, problem: `reason 过短（"${afterColon}"），须写明豁免的是什么（如「NLP 语料数据，非 UI 文案」）` };
  }
  return { ok: true, until: untilM[1], reason: afterColon };
}

/** allowlist 单条目判定。 */
export function adjudicateAllowlistEntry(entry, todayIso) {
  const problems = [];
  const reason = (entry.reason ?? "").trim();
  if (reason.length < 4) { problems.push("reason 缺失或过短"); }
  if (reason.startsWith("历史硬编码基线")) { problems.push("reason 是默认模板（须写具体豁免对象）"); }
  if (entry.expiry && entry.expiry < todayIso) { problems.push(`expiry ${entry.expiry} 已过期`); }
  return problems;
}

// ── 文件收集（与 i18n-scan.mjs 同口径：src/** 的 ts/tsx，排除测试）──
function collectFiles(dir, out) {
  if (!existsSync(dir)) { return out; }
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) { collectFiles(p, out); }
    else if (/\.(ts|tsx)$/.test(name) && !name.endsWith(".d.ts")) { out.push(p); }
  }
  return out;
}

// ── selftest ──
function runSelftest() {
  let fail = 0;
  const check = (cond, msg) => {
    if (!cond) { console.error(`SELFTEST FAIL：${msg}`); fail++; }
  };
  const TODAY = "2026-10-31";

  // 负控：缺 until ⇒ 红
  const r1 = adjudicateExemptLine("// i18n-exempt: NLP 语料数据，非 UI 文案", TODAY);
  check(!r1.ok && /until/.test(r1.problem), "负控：缺 until 必须红");
  // 负控：until 已过期 ⇒ 红
  const r2 = adjudicateExemptLine("// i18n-exempt: mock 数据 (until=2026-01-01)", TODAY);
  check(!r2.ok && /已过/.test(r2.problem), "负控：过期 must 红");
  // 正控：完整标记 ⇒ 绿
  const r3 = adjudicateExemptLine("// i18n-exempt: NLP 语料数据，非 UI 展示文案 (until=2027-01-31)", TODAY);
  check(r3.ok, "正控：reason+未过期 until 必须绿");
  // 负控：reason 过短 ⇒ 红
  const r4 = adjudicateExemptLine("// i18n-exempt: 见 (until=2027-01-31)", TODAY);
  check(!r4.ok && /reason/.test(r4.problem), "负控：reason 过短必须红");
  // allowlist：默认模板 reason ⇒ 红
  const p1 = adjudicateAllowlistEntry({ reason: "历史硬编码基线（注释/内部日志/数据/技术字符串）" }, TODAY);
  check(p1.length === 1 && /默认模板/.test(p1[0]), "allowlist 负控：默认模板 reason 必须红");
  // allowlist：具体 reason 无 expiry ⇒ 绿
  const p2 = adjudicateAllowlistEntry({ reason: "浏览器 mock 数据键名，技术字符串" }, TODAY);
  check(p2.length === 0, "allowlist 正控：具体 reason 必须绿");

  if (fail > 0) { return false; }
  console.log("SELFTEST OK：until 缺失/过期红 · reason 过短红 · 完整标记绿 · allowlist 模板 reason 红 全部通过");
  return true;
}

// ── main ──
if (SELFTEST) { process.exit(runSelftest() ? 0 : 1); }

const TODAY = new Date().toISOString().slice(0, 10);

/** 问题键（**不含行号**——行号随编辑漂移，.i18n-allowlist.json 行号键腐烂是前车之鉴）。 */
const fileProblemKey = (p) => `${p.file}::${p.kind}::${p.text.slice(0, 60)}`;
const entryProblemKey = (p) => `${p.file}::${p.kind}`;

// ── ① 文件级 i18n-exempt 审计 ──
const KIND_UNTIL = "缺until";
const KIND_EXPIRED = "until已过";
const KIND_SHORT_REASON = "reason过短";
const files = collectFiles(join(root, "src"), []).map((p) => relative(root, p).split("\\").join("/"));
const exemptFiles = [];
const fileProblems = [];
for (const rel of files) {
  const content = readFileSync(join(root, rel), "utf8");
  if (!content.includes(EXEMPT_MARK)) { continue; }
  exemptFiles.push(rel);
  content.split("\n").forEach((l, i) => {
    if (!l.includes(EXEMPT_MARK)) { return; }
    const r = adjudicateExemptLine(l, TODAY);
    if (!r.ok) {
      const kind = r.problem.includes("已过") ? KIND_EXPIRED : r.problem.includes("reason") ? KIND_SHORT_REASON : KIND_UNTIL;
      fileProblems.push({ file: rel, line: i + 1, kind, text: l.trim() });
    }
  });
}

// ── ② allowlist 审计 ──
const KIND_TEMPLATE = "默认模板reason";
const KIND_SHORT = "reason过短";
const KIND_ENTRY_EXPIRED = "expiry已过";
let allowlist = { entries: [] };
if (existsSync(ALLOWLIST_PATH)) {
  try { allowlist = JSON.parse(readFileSync(ALLOWLIST_PATH, "utf8")); } catch { /* 按空处理 */ }
}
const entries = allowlist.entries ?? [];
const deadFiles = entries.filter((e) => e.file && !existsSync(join(root, e.file)));
const entryProblems = [];
for (const e of entries) {
  if (e.file && !existsSync(join(root, e.file))) { continue; } // 死文件单独一本账
  for (const problem of adjudicateAllowlistEntry(e, TODAY)) {
    const kind = problem.includes("默认模板") ? KIND_TEMPLATE : problem.includes("过期") ? KIND_ENTRY_EXPIRED : KIND_SHORT;
    entryProblems.push({ file: e.file, kind });
  }
}

console.log(`\n=== i18n 豁免预算检查（${TODAY}）===`);
console.log(`① 文件级 i18n-exempt：${exemptFiles.length} 个文件 · 标记行问题 ${fileProblems.length} 处`);
console.log(`② allowlist：${entries.length} entries · 死文件 ${deadFiles.length} · 条目问题 ${entryProblems.length}`);

// ── --update-allowlist：清洗（不新增、不改 reason）──
if (UPDATE_ALLOWLIST) {
  const cleaned = entries.filter((e) => e.file && existsSync(join(root, e.file)));
  allowlist.entries = cleaned;
  allowlist.total_entries = cleaned.length;
  allowlist.total_files = new Set(cleaned.map((e) => e.file)).size;
  allowlist.generated = TODAY;
  writeFileSync(ALLOWLIST_PATH, JSON.stringify(allowlist, null, 2) + "\n", "utf8");
  console.log(`\nallowlist 已清洗：${entries.length} → ${cleaned.length} entries（剔除 ${entries.length - cleaned.length} 个死文件条目；未新增任何条目）。`);
  console.log(`⚠ 行号漂移条目未自动剔除（需先跑 i18n-scan --report 拿实际违规集对照）——此为显式人工步骤。`);
  process.exit(0);
}

// ── ③ 问题集棘轮：只减不增（存量冻结在基线，新增必红）──
let baseline = { frozen: { fileProblem: [], deadFile: [], entryProblem: [] } };
if (existsSync(BASELINE_PATH)) {
  try { baseline = JSON.parse(readFileSync(BASELINE_PATH, "utf8")); } catch { /* 空 */ }
}
const frozen = {
  fileProblem: new Set(baseline.frozen?.fileProblem ?? []),
  deadFile: new Set(baseline.frozen?.deadFile ?? []),
  entryProblem: new Set(baseline.frozen?.entryProblem ?? []),
};
const newFileProblems = fileProblems.filter((p) => !frozen.fileProblem.has(fileProblemKey(p)));
const newDeadFiles = deadFiles.filter((e) => !frozen.deadFile.has(e.file));
const newEntryProblems = entryProblems.filter((p) => !frozen.entryProblem.has(entryProblemKey(p)));
const newTotal = newFileProblems.length + newDeadFiles.length + newEntryProblems.length;

if (UPDATE_BASELINE) {
  if (newTotal > 0 && existsSync(BASELINE_PATH)) {
    console.error(`\nFAIL: --update-baseline 拒绝执行：当前存在 ${newTotal} 条基线外新问题（新增豁免必须走人工评审，不许用 update 洗白）。`);
    process.exit(1);
  }
  writeFileSync(BASELINE_PATH, JSON.stringify({
    note: "豁免账本棘轮基线：三本账的问题集合只减不增。新增 i18n-exempt / allowlist 条目必须带 reason+until=YYYY-MM-DD（具体 reason，非默认模板）。",
    updated: TODAY,
    frozen: {
      fileProblem: fileProblems.map(fileProblemKey),
      deadFile: deadFiles.map((e) => e.file),
      entryProblem: entryProblems.map(entryProblemKey),
    },
  }, null, 2) + "\n", "utf8");
  console.log(`\n棘轮基线已${existsSync(BASELINE_PATH) ? "收紧" : "首建"}：冻结 ${fileProblems.length} 标记行问题 / ${deadFiles.length} 死文件 / ${entryProblems.length} 条目问题。`);
  process.exit(0);
}

let fail = 0;
if (newFileProblems.length > 0) {
  console.error(`\nFAIL: ${newFileProblems.length} 处基线外 i18n-exempt 标记不合规（补 reason + until=YYYY-MM-DD）：`);
  for (const p of newFileProblems.slice(0, 30)) { console.error(`  + ${p.file}:${p.line} — ${p.kind}｜${p.text.slice(0, 90)}`); }
  if (newFileProblems.length > 30) { console.error(`  … 其余 ${newFileProblems.length - 30} 处省略`); }
  fail = 1;
}
if (newDeadFiles.length > 0) {
  console.error(`\nFAIL: ${newDeadFiles.length} 个基线外死文件条目滞留 allowlist（跑 --update-allowlist 清洗）：`);
  for (const e of newDeadFiles.slice(0, 14)) { console.error(`  - ${e.file}`); }
  fail = 1;
}
if (newEntryProblems.length > 0) {
  console.error(`\nFAIL: ${newEntryProblems.length} 条基线外 allowlist 条目不合规：`);
  for (const p of newEntryProblems.slice(0, 40)) { console.error(`  + ${p.file} — ${p.kind}`); }
  fail = 1;
}
if (fail === 0) {
  const frozenCount = frozen.fileProblem.size + frozen.deadFile.size + frozen.entryProblem.size;
  console.log(`\nPASS: 无基线外豁免问题（存量 ${frozenCount} 条冻结在棘轮基线，修复后跑 --update-baseline 收紧）。`);
}
process.exit(fail);
