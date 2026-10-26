#!/usr/bin/env node
/**
 * R-3b 门禁：回放 ↔ 实盘 一致性校验
 *
 * 校验「同一天两路跑」是否给出同一个答案：
 *   回放 = purpose='time_travel_replay' 且 as_of_date = D（D 日的 as-of 视角）
 *   实盘 = purpose IS NULL 且 left(created_at,10) = D（D 日跑的实时视角）
 *
 * 比三样：
 *   1) 票集合重合度：两路都出的 (stock_code, period)
 *   2) 风格一致性：回放的 style 是否 ∈ 实盘该票该档的风格集合
 *   3) 入场价偏差：|px_replay − px_live| / px_live
 *
 * ⚠⚠ 口径陷阱（2026-10-26 实测踩过，别改回去）：
 *   **风格一致性不能用「逐字相等」判** —— 实盘同一只票会被**多个风格同时命中**
 *   （实测 600664@2026-10-12：实盘落 9 个风格，其中 8 个是 conf=40 的 synthetic 兜底票，
 *   回放只落 reversion）⇒ 逐字相比必然判「不一致」，得出 11.1% 的假低值。
 *   正确判据：**回放的 style 集合 ⊆ 实盘的 style 集合**。
 *
 * 退出码：0 = PASS；1 = FAIL（一致性不足或样本不足）
 */
import { execFileSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(__dirname, '..');

// ── 阈值（可按样本量调）──
const MIN_PAIRED_TICKETS = 5; // 配对票数下限：低于此 ⇒ 报「样本不足」而不是「不一致」
const MAX_PX_DEVIATION = 0.05; // 入场价最大允许偏差（5%，与 R-3a 价源平价阈值同口径）

const sql = `
WITH live_by_ticket AS (
  SELECT
    left(created_at, 10) AS day,
    stock_code, period,
    array_agg(DISTINCT style) AS live_styles,
    array_agg(DISTINCT style) FILTER (WHERE synthetic = 0) AS live_real_styles,
    count(*) AS live_rows,
    count(*) FILTER (WHERE synthetic = 0) AS live_real_rows
  FROM reco_picks
  WHERE purpose IS NULL
    AND as_of_date IS NULL
    AND left(created_at, 10) IN (
      SELECT DISTINCT as_of_date FROM reco_picks WHERE purpose = 'time_travel_replay'
    )
  GROUP BY 1, 2, 3
),
replay_by_ticket AS (
  SELECT
    as_of_date AS day, stock_code, period,
    array_agg(DISTINCT style) AS replay_styles,
    count(*) AS replay_rows,
    count(*) FILTER (WHERE synthetic = 0) AS replay_real_rows
  FROM reco_picks
  WHERE purpose = 'time_travel_replay'
  GROUP BY 1, 2, 3
),
paired AS (
  SELECT
    r.day, r.stock_code, r.period,
    r.replay_styles, l.live_styles, l.live_real_styles,
    r.replay_rows, r.replay_real_rows, l.live_rows, l.live_real_rows
  FROM replay_by_ticket r
  JOIN live_by_ticket l
    ON l.day = r.day AND l.stock_code = r.stock_code AND l.period = r.period
)
SELECT
  day,
  count(*) AS paired_tickets,
  count(*) FILTER (WHERE replay_styles <@ live_styles) AS style_in_set,
  count(*) FILTER (WHERE replay_styles <@ coalesce(live_real_styles, '{}')) AS style_in_real_set,
  sum(live_rows) AS live_rows, sum(live_real_rows) AS live_real_rows,
  sum(replay_rows) AS replay_rows, sum(replay_real_rows) AS replay_real_rows
FROM paired
GROUP BY day
ORDER BY day;
`;

// 入场价要单独取（上面的 paired 里没带价格列，改用第二个查询更清楚）
const sql2 = `
WITH live_px AS (
  SELECT left(created_at,10) AS day, stock_code, period,
         (pick_data::jsonb->>'price')::float8 AS px
  FROM reco_picks
  WHERE purpose IS NULL AND as_of_date IS NULL
    AND pick_data::jsonb->>'price' IS NOT NULL
    AND left(created_at,10) IN (SELECT DISTINCT as_of_date FROM reco_picks WHERE purpose='time_travel_replay')
),
replay_px AS (
  SELECT as_of_date AS day, stock_code, period,
         (pick_data::jsonb->>'price')::float8 AS px
  FROM reco_picks
  WHERE purpose='time_travel_replay' AND pick_data::jsonb->>'price' IS NOT NULL
)
SELECT r.day,
       count(*) AS priced_pairs,
       count(*) FILTER (WHERE abs(r.px - l.px) / l.px <= ${MAX_PX_DEVIATION}) AS px_ok,
       round(max(abs(r.px - l.px) / l.px)::numeric, 4) AS max_px_dev
FROM replay_px r JOIN live_px l
  ON l.day=r.day AND l.stock_code=r.stock_code AND l.period=r.period
GROUP BY r.day ORDER BY r.day;
`;

function query(sqlText) {
  const out = execFileSync('node', [resolve(ROOT, 'scripts/pg-connect.mjs'), 'sql', sqlText], {
    cwd: ROOT,
    encoding: 'utf8',
    maxBuffer: 64 * 1024 * 1024,
  });
  const lines = out
    .split('\n')
    .filter((l) => l.trim() && !l.startsWith('错误'))
    .map((l) => l.replace(/\r/g, '').trimEnd());
  if (lines.length === 0) return [];
  const head = lines[0].split('\t').map((h) => h.trim());
  return lines.slice(1).map((l) => {
    const cells = l.split('\t');
    const o = {};
    head.forEach((h, i) => {
      // pg-connect 的输出可能带尾随空白/引号 ⇒ 统一清理
      o[h] = String(cells[i] ?? '').replace(/\r/g, '').trim().replace(/^"|"$/g, '');
    });
    return o;
  });
}

console.log('=== R-3b 回放 ↔ 实盘 一致性校验 ===');
const styleRows = query(sql);
const pxRows = query(sql2);
const pxByDay = new Map(pxRows.map((r) => [r.day, r]));

if (styleRows.length === 0) {
  console.log('⚠ 无「同一天两路」的可比数据（回放 as_of 与实盘跑批日无交集）⇒ 样本不足，无法判定');
  process.exit(1);
}

console.log('day\tpaired\tstyle_in_set\tstyle_pct\tpx_ok\tmax_px_dev\tlive(真/总)\treplay(真/总)');
let totalPaired = 0;
let totalStyleOk = 0;
let totalPxOk = 0;
let totalPx = 0;
let maxDev = 0;
for (const r of styleRows) {
  const px = pxByDay.get(r.day) || { px_ok: '0', max_px_dev: '0' };
  const paired = Number(r.paired_tickets);
  const styleOk = Number(r.style_in_set);
  totalPaired += paired;
  totalStyleOk += styleOk;
  totalPxOk += Number(px.px_ok);
  totalPx += Number(px.priced_pairs || 0);
  maxDev = Math.max(maxDev, Number(px.max_px_dev || 0));
  console.log(
    [
      r.day,
      paired,
      styleOk,
      `${((100 * styleOk) / paired).toFixed(1)}%`,
      `${px.px_ok}/${px.priced_pairs || 0}`,
      px.max_px_dev,
      `${r.live_real_rows}/${r.live_rows}`,
      `${r.replay_real_rows}/${r.replay_rows}`,
    ].join('\t'),
  );
}

const stylePct = totalPaired ? (100 * totalStyleOk) / totalPaired : 0;
const pxPct = totalPx ? (100 * totalPxOk) / totalPx : 0;
console.log('---');
console.log(
  `合计：配对 ${totalPaired} 票 | style 一致 ${stylePct.toFixed(1)}% | 入场价在 ${(MAX_PX_DEVIATION * 100).toFixed(0)}% 内 ${pxPct.toFixed(1)}% | 最大价偏差 ${(maxDev * 100).toFixed(2)}%`,
);

const problems = [];
if (totalPaired < MIN_PAIRED_TICKETS) {
  problems.push(`配对票数 ${totalPaired} < 下限 ${MIN_PAIRED_TICKETS}（样本不足，不能判 PASS）`);
}
if (totalPaired >= MIN_PAIRED_TICKETS && stylePct < 100) {
  problems.push(`style 一致率 ${stylePct.toFixed(1)}% < 100%`);
}
if (totalPx > 0 && pxPct < 100) {
  problems.push(`入场价一致率 ${pxPct.toFixed(1)}% < 100%`);
}
if (maxDev > MAX_PX_DEVIATION) {
  problems.push(`最大价偏差 ${(maxDev * 100).toFixed(2)}% > ${(MAX_PX_DEVIATION * 100).toFixed(0)}%`);
}

if (problems.length) {
  console.log('❌ FAIL');
  problems.forEach((p) => console.log(`  · ${p}`));
  process.exit(1);
}
console.log('✅ PASS');
