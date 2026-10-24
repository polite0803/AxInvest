#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
// 全市场清单覆盖率门（0-D / P-e，2026-10-18，PLAN-mover-coverage-and-reflection-exit）。
//
// ## 判的是什么
// `market_stock_universe` 就是**荐股候选宇宙** —— 清单里有多少只票，智能荐股就只可能看见多少只。
// 现网实测：679 行，其中深市主板 618 / 沪主板 37 / 创业板 16 / 科创板 8。
// 而扩页逻辑（`init/services.rs` 里清单扩页那段 tick 循环）每 tick 6 页、10 tick 走完 60 页，`pz=100` ⇒
// 槽位 6000，本该覆盖全市场；实际只有约 6 页成功（push2 族在本机的累积/速率触发封禁，
// 失败页在 `market_close_store.rs::refresh_universe_pages` 的失败页分支 只 `warn` 后跳过）。**覆盖率上不去 ⇒ 妖股根本进不了候选**，
// 这正是 002696 那轮归因里「回放真实荐股样本 = 0」的上游一环。
//
// ## 规则（C1 起只设方向性判据，不拍数值）
// C1 有分母才算数：最近一次 tick 的 `coverage_pct < FLOOR_PCT` ⇒ 红。
//    分母 = 接口自报的 `reported_total`（`market_universe_stats`，0-D 新表）——
//    **不写死 5400 之类外部常数**，那是第五套参数；页全失败时生产侧不落行（拿不到 ≠ 0 只）。
// C2 分市场配比：任一板块占比 > MIX_CAP_PCT 且其余板块合计 < (100-该值) ⇒ 按「枚举域偏斜」红。
//    现网深市主板 91% ⇒ 这条一上就红，红得对：它要的就是这个效果。
// C3 表里一行都没有 ⇒ 红（不是「跳过」）。0-D 写侧没生效与「本机没跑过 cron」这两种情况
//    都必须出声，静默通过等于把「没采到」演成「没问题」。
// C4 失败页占比：`pages_failed / pages_requested > FAIL_CAP` ⇒ 追加一条红（分母有了但爬不动）。
// C5 读数新鲜度：`observed_at` 距今 > FRESH_HOURS ⇒ 红。0-D 的写侧原先只在 clist 刷新时落行，
//    而 clist 页轮转曾按进程内 tick 归零 ⇒ 清单已扩到 2500+、读数还停在 684/11.5%。
//
// ⚠ **本门不注册进 CI**：CI 里没有本机 PG，`reported_total`/清单构成只能从实库取。
//   它是**本地诊断门**，跑法 `node scripts/check-universe-coverage.mjs`；
//   `--selftest` 用合成读数证明判据会红（不连库），那部分可以进任何地方。
//   这条偏离写在这里而不是悄悄少注册一次：门不进 CI 的理由必须和门的规则同等可见。
//
// 用法：node scripts/check-universe-coverage.mjs [--selftest] [--json]

import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

// P-e：方向性下限。2026-10-23 用户口径：漏检样本必须是**全 A 股**（现网 reported_total≈5925）。
// 旧值 50% 是「积累 3–5 个交易日再抬」前的临时值 —— 53% 已经意味着近一半票看不见，
// 不能再算达标；现按「全 A 股样本」抬到 95%（停牌/缺值由采集侧显式声明，不并入分母凑数）。
const FLOOR_PCT = 95;
/** 最新收盘覆盖（`market_daily_close` 最新交易日 distinct 代码 / reported_total）的下限。 */
const CLOSE_FLOOR_PCT = 95;
const MIX_CAP_PCT = 60;
/** 任一 A 股板块在清单里占比低于该值 ⇒ 判定「缺段」（创业板/科创板/北交所 不应接近 0）。 */
const SEGMENT_MIN_PCT = 1;
const SEGMENTS = ["main_sh", "main_sz", "chinext", "star", "bj"];
const FAIL_CAP = 0.5;
const FRESH_HOURS = 36;

/** 纯判据：喂数字出红条。`stats` = 最近一次 tick 行，`mix` = 分市场计数，`close` = 最新收盘读数。 */
export function judge(stats, mix, nowMs = Date.now(), close = null) {
  const reds = [];
  // C2 独立于 C1/C3：它算的是另一张表（`market_stock_universe`）的现场构成，
  // 与「有没有分母行」无关。早退会把「清单 91% 是深市主板」这条真实缺陷一起藏掉。
  const total = Object.values(mix).reduce((a, b) => a + (Number(b) || 0), 0);
  if (total > 0) {
    for (const [k, v] of Object.entries(mix)) {
      const pct = ((Number(v) || 0) / total) * 100;
      if (pct > MIX_CAP_PCT) {
        reds.push(`C2 枚举域偏斜：${k} 占 ${pct.toFixed(1)}% > 配比上限 ${MIX_CAP_PCT}%（清单 ${total} 只）`);
      }
    }
    // C7：全 A 股必须覆盖五个板块；创业板/科创板/北交所接近 0 同样是「样本残缺」。
    for (const m of SEGMENTS) {
      const n = Number(mix[m]) || 0;
      const pct = (n / total) * 100;
      if (pct < SEGMENT_MIN_PCT) {
        reds.push(
          `C7 分市场缺段：${m} 占 ${pct.toFixed(2)}%（${n} 只）< ${SEGMENT_MIN_PCT}% ⇒ 全 A 股样本不完整`,
        );
      }
    }
  } else {
    reds.push("C2 清单为空或计数解析不出，无从计算分市场配比");
  }

  // C6：最新收盘覆盖必须同样按全市场分母量；无读数与低覆盖分开说。
  if (close == null || !(Number(close.codes) > 0)) {
    reds.push("C6 market_daily_close 无最新收盘读数 ⇒ 全 A 股样本的收盘覆盖无法量");
  } else if (stats && Number(stats.reportedTotal) > 0) {
    const pct = (Number(close.codes) / Number(stats.reportedTotal)) * 100;
    if (pct < CLOSE_FLOOR_PCT) {
      reds.push(
        `C6 最新收盘 ${close.codes} 只（${close.date ?? "?"}）/ 全市场 ${stats.reportedTotal} = ${pct.toFixed(1)}% < ${CLOSE_FLOOR_PCT}% ⇒ 漏检样本缺一大半`,
      );
    }
  }

  if (!stats) {
    reds.push("C3 `market_universe_stats` 一行都没有 ⇒ 覆盖率从未被量过（0-D 写侧没生效，或 cron 从未跑过）");
    return reds;
  }
  const { coveragePct, universeSize, reportedTotal, pagesRequested, pagesFailed } = stats;
  if (!(reportedTotal > 0)) {
    reds.push("C1 落库行的分母 `reported_total` ≤ 0 —— 生产侧约定「无分母不落行」，出现此行说明约定被破");
    return reds;
  }
  if (coveragePct < FLOOR_PCT) {
    reds.push(
      `C1 清单覆盖率 ${coveragePct}% < 下限 ${FLOOR_PCT}%（universe=${universeSize} / 全市场申报 ${reportedTotal}）⇒ 荐股候选宇宙大面积不可见`,
    );
  }
  if (pagesRequested > 0 && pagesFailed / pagesRequested > FAIL_CAP) {
    reds.push(`C4 本次 tick 失败页 ${pagesFailed}/${pagesRequested} > ${FAIL_CAP * 100}% ⇒ 覆盖率修不动的直接原因`);
  }
  if (stats.observedAt != null) {
    const ageHours = (nowMs - Number(stats.observedAt)) / 3_600_000;
    if (!(ageHours <= FRESH_HOURS)) {
      reds.push(
        `C5 覆盖率读数已 ${ageHours.toFixed(1)} 小时未更新（>${FRESH_HOURS}h）⇒ 旧读数不能当现状`,
      );
    }
  }
  return reds;
}

/** 合成读数的自证：判据必须真的会红，且不被「无行 / 0 分母」蒙混成绿。 */
function selfTest() {
  const mix = { main_sz: 618, main_sh: 37, chinext: 16, star: 8 };
  const cases = [
    {
      name: "现网形态（679/约5400、深市 91%、收盘 1300）⇒ 至少三条红",
      stats: { coveragePct: 12.6, universeSize: 679, reportedTotal: 5400, pagesRequested: 6, pagesFailed: 5 },
      mix,
      close: { date: "2026-10-22", codes: 1300 },
      min: 3,
    },
    {
      name: "无行 + 配比偏斜 ⇒ C3 与 C2 必须同时红（早退会藏掉 C2）",
      stats: null,
      mix,
      close: null,
      min: 2,
      want: (r) => r.some((x) => x.startsWith("C3")) && r.some((x) => x.startsWith("C2")),
    },
    {
      name: "0 分母落库 ⇒ C1 红（拿不到被写成 0 只）",
      stats: { coveragePct: 0, universeSize: 679, reportedTotal: 0, pagesRequested: 6, pagesFailed: 6 },
      mix,
      close: { date: "2026-10-22", codes: 1300 },
      min: 1,
    },
    {
      name: "53% 清单 + 45% 收盘（2026-10-23 现网）⇒ C1/C6 必须红",
      stats: { coveragePct: 53.15, universeSize: 3149, reportedTotal: 5925, pagesRequested: 6, pagesFailed: 0 },
      mix: { main_sh: 1737, main_sz: 1206, chinext: 17, star: 8, bj: 181 },
      close: { date: "2026-10-22", codes: 2690 },
      min: 3,
      want: (r) => r.some((x) => x.startsWith("C1")) && r.some((x) => x.startsWith("C6")),
    },
    {
      name: "覆盖率达标但配比仍偏斜 ⇒ 只剩 C2",
      stats: { coveragePct: 95, universeSize: 5130, reportedTotal: 5400, pagesRequested: 6, pagesFailed: 0 },
      mix: { main_sz: 900, main_sh: 37, chinext: 16, star: 80, bj: 300 },
      close: { date: "2026-10-22", codes: 5130 },
      min: 1,
      want: (r) => r.length === 1 && r[0].startsWith("C2"),
    },
    {
      name: "读数过期 ⇒ C5 红（旧覆盖率不能当现状）",
      stats: {
        coveragePct: 95,
        universeSize: 5130,
        reportedTotal: 5400,
        pagesRequested: 6,
        pagesFailed: 0,
        observedAt: Date.now() - 48 * 3_600_000,
      },
      mix: { main_sz: 5130, main_sh: 300, chinext: 200, star: 100, bj: 100 },
      close: { date: "2026-10-22", codes: 5130 },
      min: 1,
      want: (r) => r.some((x) => x.startsWith("C5")),
    },
  ];
  let bad = 0;
  for (const c of cases) {
    const reds = judge(c.stats, c.mix, Date.now(), c.close);
    const ok = reds.length >= c.min && (!c.want || c.want(reds));
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
  // ⚠ psql 在 Windows 上回 CRLF。不剥 `\r` 的话**表头**也带着它（`n\r`），于是按 `n` 取值
  //   全是 undefined ⇒ Number() 出 NaN ⇒ JSON 把它印成 null：读数看着像「清单是空的」，
  //   实际是取数姿势自己坏了（2026-10-18 实测踩中，C2 一度因此静默失效）。
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
  let stats = null;
  let mix = {};
  let tableMissing = false;
  try {
    const rows = sql(
      "SELECT observed_at, universe_size, reported_total, coverage_pct, pages_requested, pages_failed, rows_upserted FROM market_universe_stats ORDER BY observed_at DESC LIMIT 1",
    );
    if (rows[0]) {
      stats = {
        coveragePct: Number(rows[0].coverage_pct),
        universeSize: Number(rows[0].universe_size),
        reportedTotal: Number(rows[0].reported_total),
        pagesRequested: Number(rows[0].pages_requested),
        pagesFailed: Number(rows[0].pages_failed),
        observedAt: rows[0].observed_at,
      };
    }
  } catch (e) {
    // 新表由**启动期声明式对账**建（本仓不走 migration），所以装好新代码但没重启时，
    // 表还不存在。那不是「覆盖率没问题」也不是「取数坏了」，而是 C3 的另一种形态 ⇒
    // 交给 judge 走「一行都没有」那条红，并在这里记 `tableMissing` 让读数能说清是哪一种。
    if (!/does not exist|不存在/.test(e.message)) {
      console.error(`❌ 取数失败：${e.message}`);
      process.exit(1);
    }
    tableMissing = true;
  }
  try {
    for (const m of sql("SELECT market_type, count(*) AS n FROM market_stock_universe GROUP BY market_type")) {
      mix[m.market_type] = Number(m.n);
    }
  } catch (e) {
    console.error(`❌ 清单构成取数失败：${e.message}`);
    process.exit(1);
  }
  let close = null;
  try {
    const rows = sql(
      "SELECT trade_date, count(*) AS codes FROM market_daily_close GROUP BY trade_date ORDER BY trade_date DESC LIMIT 1",
    );
    if (rows[0]) {
      close = { date: rows[0].trade_date, codes: Number(rows[0].codes) };
    }
  } catch (e) {
    if (!/does not exist|不存在/.test(e.message)) {
      console.error(`❌ 最新收盘覆盖取数失败：${e.message}`);
      process.exit(1);
    }
  }
  const reds = judge(stats, mix, Date.now(), close);
  if (tableMissing) {
    for (let i = 0; i < reds.length; i++) {
      if (reds[i].startsWith("C3")) reds[i] += "（本例具体形态：`market_universe_stats` 表还不存在 ⇒ 需重启应用让启动期对账建表）";
    }
  }
  if (process.argv.includes("--json")) {
    console.log(JSON.stringify({ stats, mix, close, reds }, null, 2));
  } else if (reds.length > 0) {
    console.error(`❌ 清单覆盖率门红 ${reds.length} 条：`);
    for (const r of reds) console.error("  - " + r);
    console.error(
      `  读数现场：${JSON.stringify(stats)} 分市场构成：${JSON.stringify(mix)} 最新收盘：${JSON.stringify(close)}`,
    );
    process.exit(1);
  } else {
    console.log(
      `✅ 清单覆盖率 ${stats.coveragePct}% ≥ 下限 ${FLOOR_PCT}%，最新收盘 ${close.codes}/${stats.reportedTotal}（${close.date}），分市场配比未越界`,
    );
  }
}

// 仅在被直接执行时跑 CLI；被 `import { judge }` 用作纯判据时不得触发取数。
// 2026-10-24：原先无条件 main() ⇒ 单测/离线复算一 import 就去连库，判据无法脱离 PG 复用。
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main();
