#!/usr/bin/env node
// 股票分析质量对比：同一批股票在两个模板版本 / 两批 analysis_id 下的差异。
//
// 用法：
//   node scripts/stock-analysis-quality-diff.mjs --a 179 --b 180
//   node scripts/stock-analysis-quality-diff.mjs --a-ids id1,id2 --b-ids id3,id4
//   node scripts/stock-analysis-quality-diff.mjs --a 179 --b 180 --json
//
// 口径：
//   - 按 (stock_code, analysis_date) 配对；不同分析日不互相比较；
//   - 动作翻转率、仓位/置信度/一致性分差、四档动作差、冲突类型、schema 完整率、
//     outcome 胜率、辩论 consensus/分歧/ack 覆盖度、风险档与分歧度；
//   - 只读，不写库、不自动改默认值。

import {
  attachNodeMetrics,
  fetchAnalyses,
  fetchNodeMetrics,
  median,
  mean,
  num,
  round1,
} from "./lib/stock-analysis-data.mjs";

function arg(name, fallback) {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}
function idsArg(name) {
  return arg(name, "")
    .split(/[,\s]+/)
    .map((s) => s.trim())
    .filter(Boolean);
}

const A = Number(arg("a", arg("baseline", "179")));
const B = Number(arg("b", arg("candidate", "180")));
const A_IDS = idsArg("a-ids");
const B_IDS = idsArg("b-ids");
const AS_JSON = process.argv.includes("--json");

const rows = fetchAnalyses({ aVersion: A, bVersion: B, aIds: A_IDS, bIds: B_IDS });
const metrics = fetchNodeMetrics([...A_IDS, ...B_IDS]);
const entries = attachNodeMetrics(rows, metrics);

const aSet = new Set(A_IDS);
const bSet = new Set(B_IDS);
const isA = (e) => (A_IDS.length ? aSet.has(e.id) : e.version === A);
const isB = (e) => (B_IDS.length ? bSet.has(e.id) : e.version === B);

const mapA = new Map();
const mapB = new Map();
for (const e of entries) {
  const key = `${e.stock}|${e.date}`;
  if (isA(e)) mapA.set(key, e);
  if (isB(e)) mapB.set(key, e);
}

function horizonMap(h) {
  if (!h) return {};
  if (Array.isArray(h)) {
    return Object.fromEntries(h.map((x) => [x.tier || x.horizon || x.key, x]));
  }
  return h;
}
function horizonAction(x) {
  return x ? x.action || x.decisionAction || x.decision_action || "" : "";
}
function horizonPosition(x) {
  return x ? num(x.positionPct ?? x.position_pct ?? x.position) : null;
}

const pairs = [];
const onlyA = [];
const onlyB = [];
for (const [key, a] of mapA) {
  const b = mapB.get(key);
  if (b) pairs.push({ key, a, b });
  else onlyA.push(a);
}
for (const [key, b] of mapB) {
  if (!mapA.has(key)) onlyB.push(b);
}

function metricSummary(list) {
  const outcomes = list.filter((e) => e.outcome === "win" || e.outcome === "loss");
  return {
    n: list.length,
    schemaCompleteness: round1(mean(list.map((e) => e.schema.pct))),
    outcomeWinRate: outcomes.length
      ? round1((outcomes.filter((e) => e.outcome === "win").length / outcomes.length) * 100)
      : null,
    consensusMedian: round1(median(list.map((e) => e.debate.consensus))),
    disputesMedian: round1(median(list.map((e) => e.debate.disputes))),
    bullAcksMedian: round1(median(list.map((e) => e.debate.bullAcks))),
    bearAcksMedian: round1(median(list.map((e) => e.debate.bearAcks))),
    disagreementMedian: round1(median(list.map((e) => e.risk.disagreement))),
  };
}

let actionFlips = 0;
let posDeltaSum = 0;
let posDeltaN = 0;
let confDeltaSum = 0;
let confDeltaN = 0;
let agreementDeltaSum = 0;
let agreementDeltaN = 0;
let horizonChanged = 0;
let independentPresent = 0;
let independentMatchesFormula = 0;
let independentMatchesLlm = 0;
let riskLevelChanges = 0;
const conflictTypes = { a: {}, b: {} };

const details = pairs.map(({ key, a, b }) => {
  const flipped = (a.action || "") !== (b.action || "");
  if (flipped) actionFlips += 1;
  let posDelta = null;
  if (a.position !== null && b.position !== null) {
    posDelta = round1(b.position - a.position);
    posDeltaSum += Math.abs(posDelta);
    posDeltaN += 1;
  }
  let confDelta = null;
  if (a.confidence !== null && b.confidence !== null) {
    confDelta = round1(b.confidence - a.confidence);
    confDeltaSum += Math.abs(confDelta);
    confDeltaN += 1;
  }
  let agreementDelta = null;
  if (a.agreementTotal !== null && b.agreementTotal !== null) {
    agreementDelta = round1(b.agreementTotal - a.agreementTotal);
    agreementDeltaSum += Math.abs(agreementDelta);
    agreementDeltaN += 1;
  }
  const ha = horizonMap(a.horizon);
  const hb = horizonMap(b.horizon);
  const tiers = new Set([...Object.keys(ha), ...Object.keys(hb)]);
  const tierChanges = [];
  for (const tier of tiers) {
    const aa = horizonAction(ha[tier]);
    const bb = horizonAction(hb[tier]);
    const ap = horizonPosition(ha[tier]);
    const bp = horizonPosition(hb[tier]);
    if (aa !== bb || (ap !== null && bp !== null && ap !== bp)) {
      tierChanges.push({ tier, actionA: aa, actionB: bb, positionA: ap, positionB: bp });
    }
  }
  if (tierChanges.length) horizonChanged += 1;
  if ((a.riskLevel || "") !== (b.riskLevel || "")) riskLevelChanges += 1;
  conflictTypes.a[a.conflictType || "none"] = (conflictTypes.a[a.conflictType || "none"] || 0) + 1;
  conflictTypes.b[b.conflictType || "none"] = (conflictTypes.b[b.conflictType || "none"] || 0) + 1;
  if (b.independentPresent) {
    independentPresent += 1;
    if (b.independentAction && b.independentAction === b.formulaAction) {
      independentMatchesFormula += 1;
    }
    if (b.independentAction && b.independentAction === b.llmAction) {
      independentMatchesLlm += 1;
    }
  }
  return {
    key,
    stock: b.stock,
    date: b.date,
    actionA: a.action,
    actionB: b.action,
    flipped,
    positionA: a.position,
    positionB: b.position,
    posDelta,
    confidenceA: a.confidence,
    confidenceB: b.confidence,
    confDelta,
    agreementA: a.agreementTotal,
    agreementB: b.agreementTotal,
    agreementDelta,
    schemaA: a.schema.pct,
    schemaB: b.schema.pct,
    formulaB: b.formulaAction,
    llmB: b.llmAction,
    independentB: b.independentAction,
    independentF7Source: b.independentF7Source,
    tierChanges,
  };
});

const summary = {
  aLabel: A_IDS.length ? "ids" : A,
  bLabel: B_IDS.length ? "ids" : B,
  paired: pairs.length,
  onlyA: onlyA.length,
  onlyB: onlyB.length,
  a: metricSummary([...mapA.values()]),
  b: metricSummary([...mapB.values()]),
  actionFlips,
  actionFlipRate: pairs.length ? round1((actionFlips / pairs.length) * 100) : null,
  meanAbsPositionDelta: posDeltaN ? round1(posDeltaSum / posDeltaN) : null,
  meanAbsConfidenceDelta: confDeltaN ? round1(confDeltaSum / confDeltaN) : null,
  meanAbsAgreementDelta: agreementDeltaN ? round1(agreementDeltaSum / agreementDeltaN) : null,
  horizonChanged,
  riskLevelChanges,
  conflictTypes,
  independentPresent,
  independentMatchFormulaRate: independentPresent
    ? round1((independentMatchesFormula / independentPresent) * 100)
    : null,
  independentMatchLlmRate: independentPresent
    ? round1((independentMatchesLlm / independentPresent) * 100)
    : null,
};

if (AS_JSON) {
  console.log(JSON.stringify({ summary, details, onlyA, onlyB }, null, 2));
  process.exit(0);
}

console.log(`质量对比：${A_IDS.length ? "A ids" : `v${A}`} → ${B_IDS.length ? "B ids" : `v${B}`}`);
console.log(JSON.stringify(summary, null, 2));
if (details.length === 0) {
  console.log("\n没有可配对的 (stock_code, analysis_date)；请确认两批跑的是同一批股票与同一分析日。");
} else {
  console.log("\n股票                 日期        动作A→B     仓位Δ   置信Δ   一致性Δ  schemaA/B  四档变化");
  for (const d of details) {
    const pad = (s, n) => String(s ?? "-").padEnd(n);
    console.log(
      `${pad(d.stock, 20)} ${pad(d.date, 10)} ` +
        `${pad(`${d.actionA}→${d.actionB}`, 10)} ` +
        `${pad(d.posDelta ?? "-", 7)} ${pad(d.confDelta ?? "-", 7)} ` +
        `${pad(d.agreementDelta ?? "-", 8)} ${pad(`${d.schemaA}/${d.schemaB}`, 10)} ` +
        `${d.tierChanges.length ? d.tierChanges.map((x) => x.tier).join(",") : "-"}`,
    );
  }
}
