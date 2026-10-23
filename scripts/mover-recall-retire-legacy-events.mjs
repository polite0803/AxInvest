#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
//
// 一次性作废「旧口径」漏检事件（2026-10-22，PLAN-mover-recall-defect-closure 阶段 2）。
//
// 判据（并集，只作用于 pending/failed）：
//   1. `mover_recall_runs.chain IS NULL` —— 分链改造前的混池轮；
//   2. `event.anchor_date <> run.date_to` —— 旧实现滚动所有窗口产生的中间日事件
//      （新实现只核最新收盘日，anchor 必须等于 date_to）；
//   3. `chain='serenity' AND period IN ('ultra_short','short')` —— A-3 后该链该档按设计不做；
//   4. `e.window_start < r.date_from` —— 窗口起点早于该轮数据起点的伪窗口
//      （历史回补的稀疏行把 28 行窗口拼到 2005/2007/2014 年）。
//
// 只改 status → excluded（不删行）；已 analyzed/reflected 的事件不动。
// 默认 dry-run，只打印分组计数；`--apply` 才写库。
//
// 用法：
//   node scripts/mover-recall-retire-legacy-events.mjs
//   node scripts/mover-recall-retire-legacy-events.mjs --apply

import { execFileSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const APPLY = process.argv.includes("--apply");

const LEGACY_PREDICATE = `e.status IN ('pending','failed') AND (
  r.chain IS NULL
  OR e.anchor_date <> r.date_to
  OR (r.chain = 'serenity' AND e.period IN ('ultra_short','short'))
  OR e.window_start < r.date_from
)`;

const DRY_SQL = `SELECT COALESCE(r.chain,'(null)') AS chain, e.period,
  (e.anchor_date <> r.date_to) AS old_anchor,
  (e.window_start < r.date_from) AS pseudo_window, count(*) AS n
FROM mover_recall_events e JOIN mover_recall_runs r ON r.id = e.run_id
WHERE ${LEGACY_PREDICATE}
GROUP BY 1,2,3,4 ORDER BY 1,2,3,4`;

const APPLY_SQL = `UPDATE mover_recall_events e
SET status='excluded', error_detail='retired: legacy gate (chain/anchor/period/pseudo_window)'
FROM mover_recall_runs r
WHERE r.id = e.run_id AND ${LEGACY_PREDICATE}`;

const REMAIN_SQL = `SELECT status, count(*) AS n FROM mover_recall_events GROUP BY 1 ORDER BY 1`;

function runSql(sql) {
  return execFileSync(process.execPath, ["scripts/pg-connect.mjs", "sql", sql], {
    cwd: ROOT,
    encoding: "utf8",
  });
}

console.log("== 旧口径事件分组（dry-run）==");
console.log(runSql(DRY_SQL));

if (!APPLY) {
  console.log("未写库。确认上述分组后加 --apply 执行。");
  process.exit(0);
}

console.log("== 执行作废 ==");
console.log(runSql(APPLY_SQL));
console.log("== 事件状态分布 ==");
console.log(runSql(REMAIN_SQL));
