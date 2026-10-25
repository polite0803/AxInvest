// 股票分析基准/质量脚本的共享数据读取层。
//
// 只读；复用 scripts/pg-connect.mjs 的 sql 子命令。所有 SQL 只接受已经过
// `safeIds` 校验的分析 id 与数字版本号，不拼接用户自由文本。

import { execFileSync } from "node:child_process";
import path from "node:path";
import process from "node:process";

const PG = path.join(process.cwd(), "scripts", "pg-connect.mjs");

export function query(sql) {
  const out = execFileSync(process.execPath, [PG, "sql", sql], {
    encoding: "utf8",
    maxBuffer: 128 * 1024 * 1024,
  });
  const lines = out.replace(/\r/g, "").split("\n").filter((l) => l.length > 0);
  if (lines.length === 0) return [];
  const header = lines[0].split("\t");
  return lines.slice(1).map((line) => {
    const cols = line.split("\t");
    const row = {};
    header.forEach((h, i) => (row[h] = cols[i] ?? ""));
    return row;
  });
}

export function parseJson(text) {
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

export function safeIds(ids) {
  return [...new Set(ids)].filter((id) => /^[A-Za-z0-9_-]{1,64}$/.test(id));
}

export function num(v) {
  const n = Number(v);
  return Number.isFinite(n) ? n : null;
}

export function mean(xs) {
  const ys = xs.filter((x) => typeof x === "number" && Number.isFinite(x));
  return ys.length ? ys.reduce((a, b) => a + b, 0) / ys.length : null;
}

export function median(xs) {
  const ys = xs.filter((x) => typeof x === "number" && Number.isFinite(x)).sort((a, b) => a - b);
  if (!ys.length) return null;
  const mid = Math.floor(ys.length / 2);
  return ys.length % 2 ? ys[mid] : (ys[mid - 1] + ys[mid]) / 2;
}

export function round1(x) {
  return typeof x === "number" && Number.isFinite(x) ? Math.round(x * 10) / 10 : null;
}

function schemaCompleteness(dj) {
  const keys = ["action", "positionPct", "confidence", "riskLevel", "agreementBreakdown"];
  const present = keys.filter((k) => dj && dj[k] !== undefined && dj[k] !== null).length;
  return {
    present,
    total: keys.length,
    pct: Math.round((present / keys.length) * 1000) / 10,
  };
}

/**
 * 读取分析行并归一化决策 JSON。
 * 传入 `aVersion/bVersion` 或 `aIds/bIds`；后者用于 2 轮 vs 3 轮这种
 * “同一模板版本、两批 analysis_id”的 A/B。
 */
export function fetchAnalyses({ aVersion, bVersion, aIds = [], bIds = [] } = {}) {
  const ids = safeIds([...aIds, ...bIds]);
  let where;
  if (ids.length) {
    where = `id IN (${ids.map((id) => `'${id}'`).join(",")})`;
  } else {
    where = `template_version IN (${Number(aVersion)}, ${Number(bVersion)})`;
  }
  const sql = `
SELECT id, stock_code, analysis_date, template_version, decision_action,
       decision_position_pct, decision_json, horizon_decisions, outcome, completed_at
FROM stock_analyses
WHERE ${where}
  AND completed_at IS NOT NULL
ORDER BY stock_code, analysis_date, template_version`;
  return query(sql).map((r) => {
    const dj = parseJson(r.decision_json) || {};
    const agreement = dj.agreementBreakdown || {};
    // v183: 键随节点改名（independentFormula → f7ReferenceFormula）—— 不改就会恒读 null，
    // 质量脚本把「前端/脚本报表读不到」显示成「对照匹配率 0/0」，那是假绿不是缺数据。
    const independent = dj.f7ReferenceFormula || null;
    const horizon = parseJson(r.horizon_decisions);
    return {
      id: r.id,
      stock: r.stock_code,
      date: r.analysis_date || "null",
      version: Number(r.template_version),
      action: r.decision_action || "",
      position: num(r.decision_position_pct),
      confidence: num(dj.confidence ?? dj.confidenceRiskAdjusted),
      riskLevel: dj.riskLevel || "",
      formulaAction: agreement.formulaAction || r.decision_action || "",
      llmAction: agreement.llmAction || "",
      agreementTotal: num(agreement.total),
      conflictType: agreement.conflictType || "",
      f7WeightPct: num(agreement.f7WeightPct),
      independentAction: independent ? independent.action || "" : "",
      independentF7Source: independent ? independent.f7Source || "" : "",
      // 生产侧自己的 f7 口径（v183 起独立性的直接证据：`analyst_votes` = 未读 LLM 决策）
      productionF7Source: dj.f7_source || dj.f7Source || "",
      independentPresent: Boolean(independent),
      horizon: horizon || null,
      outcome: r.outcome || "",
      schema: schemaCompleteness(dj),
      debate: null,
      risk: null,
    };
  });
}

/**
 * 读取每轮/风险收敛节点输出（只取 debate-convergence--mid 与 risk-convergence）。
 * 返回 Map<analysisId, {consensus, disputes, bullAcks, bearAcks, riskLevel, disagreement}>。
 *
 * 注意：同一分析可能有多条执行；按 created_at DESC 取最新一条（首个命中即最新）。
 */
export function fetchNodeMetrics(analysisIds) {
  const ids = safeIds(analysisIds);
  if (!ids.length) return new Map();
  const sql = `
SELECT substring(w.input_params from '"analysis_id"\\s*:\\s*"([^"]+)"') AS analysis_id,
       e->>'node_id' AS node_id,
       e->'output'->>'content' AS content,
       w.created_at AS created_at
FROM workflow_executions w, jsonb_array_elements(w.node_executions::jsonb) e
WHERE w.node_executions IS NOT NULL
  AND length(w.node_executions) > 1000
  AND w.input_params LIKE '%analysis_id%'
  AND e->>'node_id' IN ('debate-convergence--mid', 'risk-convergence')
  AND substring(w.input_params from '"analysis_id"\\s*:\\s*"([^"]+)"') IN (${ids
    .map((id) => `'${id}'`)
    .join(",")})
ORDER BY w.created_at DESC`;
  const out = new Map();
  const seen = new Set();
  for (const r of query(sql)) {
    const analysisId = r.analysis_id;
    const key = `${analysisId}|${r.node_id}`;
    if (!analysisId || seen.has(key)) continue;
    seen.add(key);
    const obj = parseJson(r.content) || {};
    const verdict = obj.verdict || {};
    const entry = out.get(analysisId) || {};
    if (r.node_id === "debate-convergence--mid") {
      entry.consensus = num(
        obj.consensus_score ?? verdict.consensus_score ?? obj.aggregate_prediction?.consensus_score,
      );
      entry.disputes = Array.isArray(obj.remaining_disputes)
        ? obj.remaining_disputes.length
        : Array.isArray(verdict.remaining_disputes)
          ? verdict.remaining_disputes.length
          : null;
      entry.bullAcks = Array.isArray(obj.decisive_bull_acks)
        ? obj.decisive_bull_acks.length
        : Array.isArray(verdict.decisive_bull_acks)
          ? verdict.decisive_bull_acks.length
          : null;
      entry.bearAcks = Array.isArray(obj.decisive_bear_acks)
        ? obj.decisive_bear_acks.length
        : Array.isArray(verdict.decisive_bear_acks)
          ? verdict.decisive_bear_acks.length
          : null;
    } else {
      entry.riskLevel = obj.converged_risk_level ?? verdict.converged_risk_level ?? "";
      entry.disagreement = num(obj.disagreement_score ?? verdict.disagreement_score);
    }
    out.set(analysisId, entry);
  }
  return out;
}

export function attachNodeMetrics(entries, metrics) {
  return entries.map((e) => {
    const m = metrics.get(e.id) || {};
    return { ...e, debate: m, risk: m };
  });
}
