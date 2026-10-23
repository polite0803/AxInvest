#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
// 漏检事件「伪窗口」漂移门（阶段 G-5，2026-10-23，PLAN-mover-recall-defect-closure 阶段 E）。
//
// ## 判的是什么
// `mover_recall_events.window_start` 是这只票该档窗口的**起点日**。历史上它由「该票自己的第 N 行」
// 反推（旧实现），停牌/稀疏历史会把它推到 2005 年 ⇒ 「90 日涨幅」实测是二十年的涨幅，
// 这种行会进补分析批次、进归因、进反思（现网曾积出 22 条，已由
// `mover-recall-retire-legacy-events.mjs` 置 excluded）。
// 写侧判据已在 `events_for_tier` 改成「全局交易日历反推 + 窗口内 ≥4/5 真实行覆盖」，
// 但**判据只在跑过的那条链上生效**：换库、旧轮次重算、有人回退实现 ⇒ 同样形态的行会重新出现。
// 这道门钉的是「库里现在还不干净」，不是「代码曾经不干净」。
//
// ## 规则（P1 起三条各管一种缺席，互不顶替）
// P1 存在非 excluded 事件 `window_start < 所属轮次 date_from` ⇒ 红。
//    轮次的 `date_from` 是这轮核查覆盖的最早日；比它更早的窗口起点只能来自稀疏行拼接。
//    （与 `commands/mover_recall_analysis.rs` 的 `select_events` 跳过判据逐字同源，
//    那道门管「不进补分析」，这道门管「别再写进库」。）
// P2 事件行的 `run_id` 找不到轮次（孤儿行）⇒ 红。P1 的判据对孤儿行**无法评估**，
//    不能因为「没命中 P1」就把它们算成合格。
// P3 事件表一行都没有 ⇒ 红（不是「都合格」，是「从没跑过」）。与覆盖率门 C3 同一口径。
//
// ⚠ **本门不注册进 CI 的实库检查**：CI 里没有本机 PG。`--selftest` 用合成读数自证判据会红，
//   那部分进 `ci-check.mjs` 与 `ci.yml`（与 `check-universe-coverage.mjs` 同位同形）。
//
// 用法：node scripts/check-mover-pseudo-window.mjs [--selftest] [--json]

import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

/** 纯判据：喂计数出红条。`rows` = { total, orphans, pseudo, earliest }。 */
export function judge(rows) {
  const reds = [];
  const total = Number(rows?.total) || 0;
  const orphans = Number(rows?.orphans) || 0;
  const pseudo = Number(rows?.pseudo) || 0;
  if (total === 0) {
    reds.push("P3 `mover_recall_events` 一行都没有 ⇒ 漏检核查从未产出事件（不是「都合格」，是「没跑过」）");
    return reds;
  }
  if (pseudo > 0) {
    reds.push(
      `P1 ${pseudo} 条未作废事件的 window_start 早于所属轮次 date_from ⇒ 伪窗口（${rows.earliest ?? "?"} 起）` +
        "：稀疏历史拼出的假窗口会进补分析与归因",
    );
  }
  if (orphans > 0) {
    reds.push(
      `P2 ${orphans} 条事件的 run_id 找不到轮次 ⇒ P1 判据对它们无法评估，不得算通过`,
    );
  }
  return reds;
}

/** 合成读数的自证：三条判据各管各的，且不被「没命中」蒙混成绿。 */
function selfTest() {
  const cases = [
    {
      name: "清理前的现网形态（22 条伪窗口）⇒ 只 P1 红",
      rows: { total: 375, orphans: 0, pseudo: 22, earliest: "2005-04-06" },
      want: (r) => r.length === 1 && r[0].startsWith("P1") && r[0].includes("2005-04-06"),
    },
    {
      name: "只有孤儿行 ⇒ 只 P2 红（P1 不能替它说话）",
      rows: { total: 40, orphans: 3, pseudo: 0 },
      want: (r) => r.length === 1 && r[0].startsWith("P2"),
    },
    {
      name: "空表 ⇒ 只 P3 红（不得读成「没有伪窗口」）",
      rows: { total: 0, orphans: 0, pseudo: 0 },
      want: (r) => r.length === 1 && r[0].startsWith("P3"),
    },
    {
      name: "清理后的健康态 ⇒ 全绿",
      rows: { total: 154, orphans: 0, pseudo: 0 },
      want: (r) => r.length === 0,
    },
    {
      name: "负控：伪窗口与孤儿同时在场 ⇒ P1/P2 必须各自成条（合成一条就看不出两处都要修）",
      rows: { total: 100, orphans: 1, pseudo: 1, earliest: "2014-01-02" },
      want: (r) => r.length === 2 && r.some((x) => x.startsWith("P1")) && r.some((x) => x.startsWith("P2")),
    },
  ];
  let bad = 0;
  for (const c of cases) {
    const reds = judge(c.rows);
    const ok = c.want(reds);
    if (!ok) bad += 1;
    console.log(`  ${ok ? "✅" : "❌"} ${c.name}（实得 ${reds.length} 条）`);
    if (!ok) for (const r of reds) console.log(`      · ${r}`);
  }
  if (bad > 0) {
    console.error(`❌ 自检未通过：${bad} 条`);
    process.exit(1);
  }
  console.log(`✅ 自检通过（${cases.length} 例，判据不空转）`);
}

/** 走 pg-connect 正门取数（口令不进 argv，由该脚本自己处理）。 */
function sql(q) {
  const r = spawnSync("node", ["scripts/pg-connect.mjs", "sql", q], {
    cwd: ROOT,
    encoding: "utf8",
  });
  if (r.status !== 0) throw new Error(`取数失败: ${(r.stderr || r.stdout || "").trim().slice(0, 200)}`);
  // ⚠ Windows 上回 CRLF：不剥 `\r` 会让表头带它，按列名取值全成 undefined ⇒ NaN ⇒
  //   读数看着像「库里干净」，实际是取数姿势自己坏了（check-universe-coverage 踩过）。
  const lines = r.stdout.replace(/\r/g, "").trim().split("\n").filter(Boolean);
  if (lines.length < 2) return [];
  const head = lines[0].split("\t");
  return lines.slice(1).map((l) => {
    const cells = l.split("\t");
    const o = {};
    head.forEach((h, i) => {
      o[h] = (cells[i] ?? "").trim();
    });
    return o;
  });
}

function main() {
  if (process.argv.includes("--selftest")) {
    selfTest();
    return;
  }
  // SQL 本体保持纯 ASCII：中文进 argv 会在 Windows 管道里换码表，取数会假报失败。
  let rows;
  try {
    const c = sql(
      "SELECT count(*) AS total," +
        " count(*) FILTER (WHERE r.id IS NULL) AS orphans," +
        " count(*) FILTER (WHERE r.id IS NOT NULL AND e.status <> 'excluded'" +
        " AND r.date_from IS NOT NULL AND e.window_start < r.date_from) AS pseudo" +
        " FROM mover_recall_events e LEFT JOIN mover_recall_runs r ON r.id = e.run_id",
    )[0];
    const earliest = sql(
      "SELECT min(e.window_start) AS earliest FROM mover_recall_events e" +
        " JOIN mover_recall_runs r ON r.id = e.run_id" +
        " WHERE e.status <> 'excluded' AND r.date_from IS NOT NULL AND e.window_start < r.date_from",
    )[0];
    rows = {
      total: Number(c?.total) || 0,
      orphans: Number(c?.orphans) || 0,
      pseudo: Number(c?.pseudo) || 0,
      earliest: earliest?.earliest || null,
    };
  } catch (e) {
    if (/does not exist|不存在/.test(e.message)) {
      console.error("❌ mover_recall_events / mover_recall_runs 表还不存在 ⇒ 需重启应用让启动期对账建表");
      process.exit(1);
    }
    console.error(`❌ 取数失败：${e.message}`);
    process.exit(1);
  }
  const reds = judge(rows);
  if (process.argv.includes("--json")) {
    console.log(JSON.stringify({ rows, reds }, null, 2));
  } else if (reds.length > 0) {
    console.error(`❌ 漏检伪窗口门红 ${reds.length} 条：`);
    for (const r of reds) console.error("  - " + r);
    console.error(`  读数现场：${JSON.stringify(rows)}`);
    process.exit(1);
  } else {
    console.log(`✅ 漏检事件 ${rows.total} 条：无伪窗口、无孤儿轮次引用`);
  }
}

main();
