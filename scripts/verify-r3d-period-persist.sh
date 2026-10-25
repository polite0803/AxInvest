#!/bin/bash
# R-3d 修复验收：跑 2 个 as_of 日，核对「按档落库」是否生效
# 用法：bash scripts/verify-r3d-period-persist.sh
set -u
cd "$(dirname "$0")/../src-tauri" || exit 1

BIN=./target/debug/axagent-validate-backfill.exe
DATES="${1:-2026-09-28,2026-10-21}"
LOG=../.workbuddy/tmp/r3d_run.log

echo "=== 跑批：as_of=$DATES ==="
"$BIN" --time-travel --as-of "$DATES" --tt-timeout 900 > "$LOG" 2>&1
echo "EXIT=$?"
grep -E "链A 落库 reco_picks|链 A 落库|bucket was empty" "$LOG" | tail -20

echo
echo "=== 库内核对：按 period 分布 ==="
cd ..
node scripts/pg-connect.mjs sql "
SELECT period,
       count(*) AS picks,
       count(DISTINCT stock_code) AS stocks,
       round(avg(confidence),1) AS avg_conf,
       count(*) FILTER (WHERE synthetic=0) AS real
FROM reco_picks
WHERE purpose='time_travel_replay'
  AND created_at >= (now() - interval '30 minutes')::text
GROUP BY period ORDER BY period;
" 2>&1 | head -10

echo
echo "=== 对比：修复前基线（全部 12 天）==="
node scripts/pg-connect.mjs sql "
SELECT period, count(*) AS picks, round(avg(confidence),1) AS avg_conf
FROM reco_picks WHERE purpose='time_travel_replay'
GROUP BY period ORDER BY period;
" 2>&1 | head -10
