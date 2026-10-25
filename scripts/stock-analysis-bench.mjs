#!/usr/bin/env node
// 股票分析阶段耗时基准：从 workflow_executions.node_executions 聚合。
//
// 用法：
//   node scripts/stock-analysis-bench.mjs --runs 10
//   node scripts/stock-analysis-bench.mjs --runs 20 --stock 003006
//   node scripts/stock-analysis-bench.mjs --runs 20 --version 180
//   node scripts/stock-analysis-bench.mjs --compare 179 180
//   node scripts/stock-analysis-bench.mjs --runs 10 --json --out output/bench.json
//
// 口径：
//   - 只读带 stock_code 的工作流执行；
//   - 阶段 wall = 组内最早 started_at → 最晚 completed_at；
//   - token 取节点 output.usage；工具节点没有 usage 记 0；
//   - promptStats 取 Agent 节点 output.promptStats（需 emit_prompt_stats=true）；
//   - criticalPathApproxMs = 关键阶段 maxNodeMs 之和（近似，不是严格 DAG 最长路）。

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { parseJson, query, round1, safeIds } from "./lib/stock-analysis-data.mjs";

function arg(name, fallback) {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}
const RUNS = Math.max(1, Number(arg("runs", "10")) || 10);
const STOCK = arg("stock", "");
const VERSION = arg("version", "");
const OUT = arg("out", "");
const AS_JSON = process.argv.includes("--json");
const COMPARE_IDX = process.argv.indexOf("--compare");
const COMPARE =
  COMPARE_IDX >= 0
    ? [Number(process.argv[COMPARE_IDX + 1]), Number(process.argv[COMPARE_IDX + 2])]
    : null;

const SQL_LIMIT = VERSION || STOCK || COMPARE ? Math.max(RUNS * 20, 200) : RUNS;

const STAGE_ORDER = [
  "analyst",
  "brief",
  "debate",
  "convergence",
  "value",
  "risk",
  "research",
  "llm_decision",
  "f7_reference",
  "horizon",
  "formula",
  "tail",
  "misc",
];

const CRITICAL_STAGES = [
  "analyst",
  "debate",
  "convergence",
  "value",
  "risk",
  "research",
  "llm_decision",
  "tail",
];

function stageOf(id) {
  if (/^a-/.test(id)) return "analyst";
  if (/^analyst-brief--/.test(id)) return "brief";
  if (/^debate-bull-bear--/.test(id) || /^(bull|bear)-r/.test(id)) return "debate";
  if (/^debate-convergence--/.test(id)) return "convergence";
  if (/^value-(investor|verify)--/.test(id)) return "value";
  if (/^risk-(agg|con|neu)$/.test(id) || id === "risk-convergence") return "risk";
  if (id === "research-mgr") return "research";
  if (id === "trader") return "llm_decision";
  if (id === "portfolio-mgr-f7-ref") return "f7_reference";
  if (/^pm-h-/.test(id) || id === "pm-arbiter") return "horizon";
  if (id === "portfolio-mgr" || id === "portfolio-risk-gate") return "formula";
  if (
    id === "rule-check" ||
    id === "quality-gate" ||
    id === "decision-explainer" ||
    /^notify/.test(id) ||
    /^store/.test(id)
  ) {
    return "tail";
  }
  return "misc";
}

function fetchVersions(ids) {
  const safe = safeIds(ids);
  if (!safe.length) return new Map();
  const sql = `SELECT id, template_version FROM stock_analyses WHERE id IN (${safe
    .map((id) => `'${id}'`)
    .join(",")})`;
  return new Map(query(sql).map((r) => [r.id, Number(r.template_version)]));
}

function buildSql(limit) {
  return `
WITH recent AS (
  SELECT id, input_params, node_executions
  FROM workflow_executions
  WHERE node_executions IS NOT NULL
    AND length(node_executions) > 1000
    AND input_params LIKE '%stock_code%'
  ORDER BY created_at DESC
  LIMIT ${limit}
)
SELECT r.id AS exec_id,
       r.input_params AS input_params,
       e->>'node_id' AS node_id,
       e->>'node_type' AS node_type,
       COALESCE((e->>'execution_time_ms')::int, 0) AS ms,
       COALESCE((e->>'started_at')::bigint, 0) AS started_at,
       COALESCE((e->>'completed_at')::bigint, 0) AS completed_at,
       COALESCE((e->'output'->'usage'->>'input_tokens')::int, 0) AS in_tok,
       COALESCE((e->'output'->'usage'->>'output_tokens')::int, 0) AS out_tok,
       COALESCE(length((e->'input')::text), 0) AS input_len,
       COALESCE(length(e->'output'->>'content'), 0) AS content_len,
       e->'output'->'promptStats' AS prompt_stats
FROM recent r, jsonb_array_elements(r.node_executions::jsonb) e`;
}

function collect({ version = "" } = {}) {
  const rows = query(buildSql(SQL_LIMIT));
  const all = [];
  for (const r of rows) {
    const input = parseJson(r.input_params) || {};
    if (STOCK && input.stock_code !== STOCK) continue;
    all.push({ row: r, input });
  }
  const versionMap = version ? fetchVersions(all.map((x) => x.input.analysis_id)) : new Map();
  const filtered = version
    ? all.filter((x) => versionMap.get(x.input.analysis_id) === Number(version))
    : all;

  const byExec = new Map();
  for (const { row: r, input } of filtered) {
    const execId = r.exec_id;
    if (!byExec.has(execId)) {
      byExec.set(execId, { execId, stock: input.stock_code || "", stages: new Map(), nodes: [] });
    }
    const rec = byExec.get(execId);
    const stage = stageOf(r.node_id);
    const ms = Number(r.ms) || 0;
    const started = Number(r.started_at) || 0;
    const completed = Number(r.completed_at) || 0;
    const inTok = Number(r.in_tok) || 0;
    const outTok = Number(r.out_tok) || 0;
    const inputLen = Number(r.input_len) || 0;
    const contentLen = Number(r.content_len) || 0;
    const promptStats = parseJson(r.prompt_stats);
    rec.nodes.push({
      node: r.node_id,
      type: r.node_type,
      stage,
      ms,
      inputLen,
      contentLen,
      inTok,
      outTok,
      promptStats,
    });
    if (!rec.stages.has(stage)) {
      rec.stages.set(stage, {
        stage,
        minStart: started || Number.POSITIVE_INFINITY,
        maxEnd: completed,
        sumMs: 0,
        maxNodeMs: 0,
        nodes: 0,
        inTok: 0,
        outTok: 0,
        maxInputLen: 0,
        maxContentLen: 0,
      });
    }
    const s = rec.stages.get(stage);
    if (started > 0) s.minStart = Math.min(s.minStart, started);
    s.maxEnd = Math.max(s.maxEnd, completed);
    s.sumMs += ms;
    s.maxNodeMs = Math.max(s.maxNodeMs, ms);
    s.nodes += 1;
    s.inTok += inTok;
    s.outTok += outTok;
    s.maxInputLen = Math.max(s.maxInputLen, inputLen);
    s.maxContentLen = Math.max(s.maxContentLen, contentLen);
  }

  const result = [];
  for (const rec of byExec.values()) {
    const stages = {};
    let criticalPathApproxMs = 0;
    for (const stage of STAGE_ORDER) {
      const s = rec.stages.get(stage);
      if (!s) continue;
      const wall =
        Number.isFinite(s.minStart) && s.maxEnd > 0 ? Math.max(0, s.maxEnd - s.minStart) : 0;
      stages[stage] = {
        wallMs: wall,
        sumNodeMs: s.sumMs,
        maxNodeMs: s.maxNodeMs,
        nodes: s.nodes,
        inTokens: s.inTok,
        outTokens: s.outTok,
        maxInputChars: s.maxInputLen,
        maxOutputChars: s.maxContentLen,
      };
      if (CRITICAL_STAGES.includes(stage)) criticalPathApproxMs += s.maxNodeMs;
    }
    const starts = [...rec.stages.values()]
      .map((s) => s.minStart)
      .filter((x) => Number.isFinite(x));
    const ends = [...rec.stages.values()].map((s) => s.maxEnd).filter((x) => x > 0);
    result.push({
      execId: rec.execId,
      stock: rec.stock,
      nodeCount: rec.nodes.length,
      totalWallMs: starts.length && ends.length ? Math.max(0, Math.max(...ends) - Math.min(...starts)) : 0,
      criticalPathApproxMs,
      slowestNodes: rec.nodes.slice().sort((a, b) => b.ms - a.ms).slice(0, 8),
      debatePrompts: rec.nodes
        .filter((n) => n.stage === "debate" && n.promptStats)
        .map((n) => ({ node: n.node, ...n.promptStats })),
      stages,
    });
  }
  return {
    meta: { version: version || "all", stock: STOCK || "all", requested: RUNS, kept: result.length },
    executions: result,
  };
}

function printReport(collection) {
  const { meta, executions } = collection;
  console.log(
    `股票分析基准：requested=${meta.requested} kept=${meta.kept} version=${meta.version} stock=${meta.stock}`,
  );
  for (const r of executions) {
    console.log(
      `\n== ${r.stock || "?"} / ${r.execId} / ${r.nodeCount} nodes / ~${round1(
        r.totalWallMs / 1000,
      )}s / critical≈${round1(r.criticalPathApproxMs / 1000)}s`,
    );
    for (const stage of STAGE_ORDER) {
      const s = r.stages[stage];
      if (!s) continue;
      console.log(
        `  ${stage.padEnd(20)} wall=${String(round1(s.wallMs / 1000)).padStart(7)}s ` +
          `maxNode=${String(round1(s.maxNodeMs / 1000)).padStart(6)}s ` +
          `nodes=${String(s.nodes).padStart(3)} ` +
          `inTok=${s.inTokens} outTok=${s.outTokens} ` +
          `maxIn=${s.maxInputChars}B maxOut=${s.maxOutputChars}B`,
      );
    }
    if (r.debatePrompts.length) {
      console.log(
        "  prompts: " +
          r.debatePrompts
            .map(
              (p) =>
                `${p.node}(sys=${p.systemChars ?? "?"},user=${p.userChars ?? "?"},prefetch=${p.prefetchChars ?? "?"})`,
            )
            .join(", "),
      );
    }
    console.log(
      "  slowest: " +
        r.slowestNodes
          .map((n) => `${n.node}(${round1(n.ms / 1000)}s,in=${n.inputLen}B,out=${n.contentLen}B)`)
          .join(", "),
    );
  }
}

function stageAverages(collection) {
  const out = {};
  for (const r of collection.executions) {
    for (const [stage, s] of Object.entries(r.stages)) {
      const e = out[stage] || (out[stage] = { n: 0, wall: 0, maxNode: 0, inTok: 0, outTok: 0 });
      e.n += 1;
      e.wall += s.wallMs;
      e.maxNode += s.maxNodeMs;
      e.inTok += s.inTokens;
      e.outTok += s.outTokens;
    }
  }
  for (const e of Object.values(out)) {
    e.avgWallMs = e.n ? e.wall / e.n : 0;
    e.avgMaxNodeMs = e.n ? e.maxNode / e.n : 0;
    e.avgInTok = e.n ? e.inTok / e.n : 0;
    e.avgOutTok = e.n ? e.outTok / e.n : 0;
  }
  return out;
}

function printCompare(a, b) {
  const sa = stageAverages(a);
  const sb = stageAverages(b);
  console.log(
    `阶段对比：A(v${a.meta.version}, n=${a.meta.kept}) → B(v${b.meta.version}, n=${b.meta.kept})`,
  );
  console.log("stage                 A_wall_s  B_wall_s  Δ_s      A_maxNode_s B_maxNode_s");
  for (const stage of STAGE_ORDER) {
    if (!sa[stage] && !sb[stage]) continue;
    const aw = (sa[stage]?.avgWallMs || 0) / 1000;
    const bw = (sb[stage]?.avgWallMs || 0) / 1000;
    const an = (sa[stage]?.avgMaxNodeMs || 0) / 1000;
    const bn = (sb[stage]?.avgMaxNodeMs || 0) / 1000;
    console.log(
      `${stage.padEnd(20)} ${String(round1(aw)).padStart(8)} ${String(round1(bw)).padStart(9)} ` +
        `${String(round1(bw - aw)).padStart(8)} ${String(round1(an)).padStart(10)} ${String(round1(bn)).padStart(11)}`,
    );
  }
}

let payload;
if (COMPARE) {
  const [va, vb] = COMPARE;
  const a = collect({ version: va });
  const b = collect({ version: vb });
  payload = { a, b };
  if (AS_JSON) console.log(JSON.stringify(payload, null, 2));
  else printCompare(a, b);
} else {
  payload = collect({ version: VERSION });
  if (AS_JSON) console.log(JSON.stringify(payload, null, 2));
  else printReport(payload);
}

if (OUT) {
  fs.mkdirSync(path.dirname(OUT), { recursive: true });
  fs.writeFileSync(OUT, JSON.stringify(payload, null, 2), "utf8");
  console.log(`\n已写入 ${OUT}`);
}
