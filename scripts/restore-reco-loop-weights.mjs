#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
//
// 一次性修正 reco-loop 的复利衰减权重（PLAN: 修复一 / 拍板 5）。
//
// 背景：旧实现把「上一轮已含惩罚的生效权重」当下一轮 EWMA 的输入，同一批证据
// 每天重算一次就把 mover/熔断/负 IC 再乘一次 ⇒ capital long 0.45→0.26→0.19→0.17、
// watchlist 1→0.05。修复后的闭环按 base_weight 计算；本脚本只追加一行
// base=1.0 / outer=1.0 / new=1.0 的纠正行，让下一轮从基线重算，**不删除、不改写历史行**。
//
// 用法：
//   node scripts/restore-reco-loop-weights.mjs            # 只打印将被重置的格
//   node scripts/restore-reco-loop-weights.mjs --apply    # 追加纠正行
//
// 前置：应用已用新代码重启过（strategy_weight_history 已有 base_weight/outer_penalty 列）。

import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const apply = process.argv.includes("--apply");

function sql(query) {
  return execFileSync(
    process.execPath,
    [resolve(repoRoot, "scripts/pg-connect.mjs"), "sql", query],
    { cwd: repoRoot, encoding: "utf8" },
  );
}

const cols = sql(
  "SELECT count(*) AS n FROM information_schema.columns " +
    "WHERE table_name='strategy_weight_history' AND column_name IN ('base_weight','outer_penalty')",
);
if (!/\b2\b/.test(cols)) {
  console.error(
    "缺少 base_weight / outer_penalty 列：先用新代码重启一次应用让声明式对账建列，再跑本脚本。",
  );
  process.exit(2);
}

const latest = sql(
  "SELECT DISTINCT ON (strategy_id, period) strategy_id, period, new_weight " +
    "FROM strategy_weight_history WHERE trigger='reco-loop' " +
    "ORDER BY strategy_id, period, applied_at DESC",
);
console.log("当前最新 reco-loop 格：\n" + latest);

if (!apply) {
  console.log("dry-run：未写库。确认后加 --apply。");
  process.exit(0);
}

const cte =
  "WITH latest AS (" +
  "  SELECT DISTINCT ON (strategy_id, period) strategy_id, period, new_weight " +
  "  FROM strategy_weight_history WHERE trigger='reco-loop' " +
  "  ORDER BY strategy_id, period, applied_at DESC" +
  ") " +
  "INSERT INTO strategy_weight_history " +
  "  (id, strategy_id, period, old_weight, base_weight, outer_penalty, new_weight, " +
  "   delta_pct, trigger, source_reflection_id, sample_size, win_rate, rationale, applied_at) " +
  "SELECT 'reset-' || md5(random()::text || clock_timestamp()::text), " +
  "  strategy_id, period, new_weight, 1.0, 1.0, 1.0, " +
  "  0, 'reco-loop', NULL, 0, 0, 'one-time reset after compounding fix', " +
  "  (extract(epoch from now()) * 1000)::bigint " +
  "FROM latest";
const out = sql(cte);
console.log("已追加纠正行：\n" + out);
console.log("下一轮 07:00 重算将从 base=1.0 重新收敛（不再复利）。");
