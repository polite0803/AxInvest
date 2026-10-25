#!/usr/bin/env node
// 辩论轮次 A/B：2 轮 vs 3 轮（两批 analysis_id，因为改 debate_rounds 不换 template_version）。
//
// 用法：
//   node scripts/stock-analysis-debate-ab.mjs \
//     --a-ids <2轮批次 id...> --b-ids <3轮批次 id...>
//
// 判据（PLAN P1-B，全部写死在本脚本）：
//   - 配对样本 ≥ --min-pairs（默认 10，计划目标 20）；
//   - 动作翻转率 ≤ --max-flip（默认 0.15）；
//   - |consensus 中位差| ≤ --max-consensus-diff（默认 5）；
//   - 剩余分歧覆盖：B 中位数 / A 中位数 ≥ --min-disputes-ratio（默认 0.9）；
//     A 中位数为 0 时，要求 B ≥ A；
//   - schema 完整率 B ≥ A；
//   - outcome 胜率 B ≥ A - 5pt（无标签时记 unknown，不因此判失败）。
//
// 本脚本只出结论，不自动改 DEFAULT_DEBATE_ROUNDS。

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import {
  attachNodeMetrics,
  fetchAnalyses,
  fetchNodeMetrics,
  mean,
  median,
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

const A_IDS = idsArg("a-ids");
const B_IDS = idsArg("b-ids");
const MIN_PAIRS = Number(arg("min-pairs", "20"));
const MAX_FLIP = Number(arg("max-flip", "0.15"));
const MAX_CONSENSUS_DIFF = Number(arg("max-consensus-diff", "5"));
const MIN_DISPUTES_RATIO = Number(arg("min-disputes-ratio", "0.9"));
const AS_JSON = process.argv.includes("--json");
const OUT = arg("out", "");

if (!A_IDS.length || !B_IDS.length) {
  console.error(
    "需要两批 analysis_id：--a-ids <2轮批次> --b-ids <3轮批次>\n" +
      "（改 debate_rounds 后重新播种/跑批，模板版本可能相同，故不能用 --version 配对）",
  );
  process.exit(1);
}

const aSet = new Set(A_IDS);
const bSet = new Set(B_IDS);
const entries = attachNodeMetrics(
  fetchAnalyses({ aIds: A_IDS, bIds: B_IDS }),
  fetchNodeMetrics([...A_IDS, ...B_IDS]),
);

const mapA = new Map();
const mapB = new Map();
for (const e of entries) {
  const key = `${e.stock}|${e.date}`;
  if (aSet.has(e.id)) mapA.set(key, e);
  if (bSet.has(e.id)) mapB.set(key, e);
}

const pairs = [];
for (const [key, a] of mapA) {
  const b = mapB.get(key);
  if (b) pairs.push({ key, a, b });
}

function metrics(list) {
  const outcomes = list.filter((e) => e.outcome === "win" || e.outcome === "loss");
  return {
    n: list.length,
    schemaCompleteness: round1(mean(list.map((e) => e.schema.pct))),
    outcomeWinRate: outcomes.length
      ? round1((outcomes.filter((e) => e.outcome === "win").length / outcomes.length) * 100)
      : null,
    consensusMedian: round1(median(list.map((e) => e.debate.consensus))),
    disputesMedian: round1(median(list.map((e) => e.debate.disputes))),
  };
}

const aMetrics = metrics([...mapA.values()]);
const bMetrics = metrics([...mapB.values()]);

let flips = 0;
let posSum = 0;
let posN = 0;
let confSum = 0;
let confN = 0;
let agreementSum = 0;
let agreementN = 0;
let riskLevelChanges = 0;
for (const { a, b } of pairs) {
  if ((a.action || "") !== (b.action || "")) flips += 1;
  if (a.position !== null && b.position !== null) {
    posSum += Math.abs(b.position - a.position);
    posN += 1;
  }
  if (a.confidence !== null && b.confidence !== null) {
    confSum += Math.abs(b.confidence - a.confidence);
    confN += 1;
  }
  if (a.agreementTotal !== null && b.agreementTotal !== null) {
    agreementSum += Math.abs(b.agreementTotal - a.agreementTotal);
    agreementN += 1;
  }
  if ((a.riskLevel || "") !== (b.riskLevel || "")) riskLevelChanges += 1;
}

const flipRate = pairs.length ? flips / pairs.length : null;
const consensusDiff =
  aMetrics.consensusMedian !== null && bMetrics.consensusMedian !== null
    ? Math.abs(bMetrics.consensusMedian - aMetrics.consensusMedian)
    : null;
const disputesRatio =
  aMetrics.disputesMedian !== null && bMetrics.disputesMedian !== null
    ? aMetrics.disputesMedian > 0
      ? bMetrics.disputesMedian / aMetrics.disputesMedian
      : bMetrics.disputesMedian >= aMetrics.disputesMedian
        ? 1
        : 0
    : null;
const outcomeDelta =
  aMetrics.outcomeWinRate !== null && bMetrics.outcomeWinRate !== null
    ? bMetrics.outcomeWinRate - aMetrics.outcomeWinRate
    : null;
const schemaDelta =
  aMetrics.schemaCompleteness !== null && bMetrics.schemaCompleteness !== null
    ? bMetrics.schemaCompleteness - aMetrics.schemaCompleteness
    : null;

const criteria = {
  enough_pairs: {
    pass: pairs.length >= MIN_PAIRS,
    value: pairs.length,
    need: `>=${MIN_PAIRS}`,
  },
  action_flip: {
    pass: flipRate !== null && flipRate <= MAX_FLIP,
    value: flipRate === null ? null : round1(flipRate * 100),
    need: `<=${round1(MAX_FLIP * 100)}%`,
  },
  consensus_stability: {
    pass: consensusDiff !== null && consensusDiff <= MAX_CONSENSUS_DIFF,
    value: consensusDiff,
    need: `<=${MAX_CONSENSUS_DIFF}`,
  },
  disputes_coverage: {
    pass: disputesRatio !== null && disputesRatio >= MIN_DISPUTES_RATIO,
    value: disputesRatio === null ? null : round1(disputesRatio * 100),
    need: `>=${round1(MIN_DISPUTES_RATIO * 100)}%`,
  },
  schema_not_worse: {
    pass: schemaDelta !== null ? schemaDelta >= 0 : null,
    value: schemaDelta,
    need: ">=0",
  },
  outcome_not_worse: {
    pass: outcomeDelta !== null ? outcomeDelta >= -5 : null,
    value: outcomeDelta,
    need: ">=-5pt（无标签=unknown）",
  },
};

const hardKeys = [
  "enough_pairs",
  "action_flip",
  "consensus_stability",
  "disputes_coverage",
  "schema_not_worse",
];
const overall =
  hardKeys.every((k) => criteria[k].pass === true) &&
  (criteria.outcome_not_worse.pass === null || criteria.outcome_not_worse.pass === true);

const summary = {
  pairs: pairs.length,
  onlyA: [...mapA.keys()].filter((k) => !mapB.has(k)).length,
  onlyB: [...mapB.keys()].filter((k) => !mapA.has(k)).length,
  a: aMetrics,
  b: bMetrics,
  actionFlips: flips,
  actionFlipRate: flipRate === null ? null : round1(flipRate * 100),
  meanAbsPositionDelta: posN ? round1(posSum / posN) : null,
  meanAbsConfidenceDelta: confN ? round1(confSum / confN) : null,
  meanAbsAgreementDelta: agreementN ? round1(agreementSum / agreementN) : null,
  riskLevelChanges,
  criteria,
  overall: pairs.length ? overall : false,
};

if (AS_JSON) {
  console.log(JSON.stringify(summary, null, 2));
} else {
  console.log("辩论轮次 A/B（A=2 轮，B=3 轮）");
  console.log(JSON.stringify(summary, null, 2));
  console.log("\n判据：");
  for (const [k, v] of Object.entries(criteria)) {
    const status = v.pass === null ? "UNKNOWN" : v.pass ? "PASS" : "FAIL";
    console.log(`  ${status.padEnd(7)} ${k.padEnd(22)} value=${v.value} need=${v.need}`);
  }
  console.log(`\n结论：${summary.overall ? "允许把默认轮数改 2（仍需人工确认）" : "不通过，保留 3 轮"}`);
}

if (OUT) {
  fs.mkdirSync(path.dirname(OUT), { recursive: true });
  fs.writeFileSync(OUT, JSON.stringify(summary, null, 2), "utf8");
  console.log(`\n已写入 ${OUT}`);
}
