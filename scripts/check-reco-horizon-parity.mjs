#!/usr/bin/env node
/**
 * 荐股链四周期「口径一致」等式门禁（`PLAN-reco-horizon-science-alignment.md` Phase R-A）。
 *
 * ## 为什么需要它
 *
 * 荐股链（`src-tauri/crates/analysis-engine/src/recommender/`）的四档口径此前有四处
 * 「两处各自都自洽、两边打架」的形态，`cargo check` / `typecheck` 全绿也照样存在：
 *
 * | 门 | 锁什么 | 修复前的真实形态（本门必须对它报红） |
 * |---|---|---|
 * | **a 天数唯一来源** | `recommender/strategies/` 里出现 `holding_days` 的行必须同时是 `default_holding_days()` | `strategies/capital.rs` 自带逐档天数（short=**7**）、`watchlist.rs` 读 `wl_short_holding_days` ⇒ 与 `harness::holding_period::default_holding_days`（short=**5**）冲突，反思/回测按 5 天判成熟而荐股按 7 天 |
 * | **b 仓位口径唯一** | `period.factor()` 只允许出现在**名字含 `fallback` 的函数**里 | `scoring.rs` 的 `calc_position_with_consistency()` 无条件乘 `factor()`（0.4/0.6/0.8/1.0 经验数），与分析链的「风险预算 + 凯利」不同源 |
 * | **c 校准数不得拍脑袋** | `recommender/` 下禁止 `"neutral"` 死传、`/ 0.50` 固定基准、`=> 0.85` 反身性字面量 | `mod.rs` 三处硬编码（调用侧传死 market regime、胜率基准写死 0.50、只有超短打 0.85 折） |
 * | **d 档位序单源** | 前端手写档位序必须与后端 `Period::ALL` 逐项同序 | 展示序是消费方契约：`serde_json` 的 map 按字符串字典序排，**不随变体序**（本仓实测） |
 *
 * **扫描面非零**是第 0 条判据：任何一门解析到 0 命中即 exit 2 —— 「扫到 0 条」的门等于没有门
 * （本仓已多次为这类假绿付过代价，判据同 `check-horizon-weight-parity.mjs:29`）。
 *
 * 用法：
 *   node scripts/check-reco-horizon-parity.mjs            # 比对（有违规 exit 1）
 *   node scripts/check-reco-horizon-parity.mjs --selftest # 正负对照（复刻修复前形态，证明它会红）
 */

import fs from "node:fs";
import path from "node:path";
import process from "node:process";

const ROOT = path.resolve(import.meta.dirname, "..");
const STRATEGIES_DIR = "src-tauri/crates/analysis-engine/src/recommender/strategies";
const SCORING = "src-tauri/crates/analysis-engine/src/recommender/scoring.rs";
const RECO_MOD = "src-tauri/crates/analysis-engine/src/recommender/mod.rs";
const MATRIX_SRC = "src-tauri/crates/analysis-engine/src/recommender/style_matrix.rs";
const HOLDING = "src-tauri/crates/harness/src/holding_period.rs";
const PANEL = "src/components/stock-analysis/RecommendationPanel.tsx";
const MATRIX = "src/components/stock-analysis/RecoStrategyMatrix.tsx";

const read = (rel) => fs.readFileSync(path.join(ROOT, rel), "utf8");

/** 门 a：策略里任何 `holding_days` 写入必须来自 `default_holding_days()`。 */
function checkHoldingDaysSingleSource(files) {
  const violations = [];
  let scanned = 0;
  for (const { name, src } of files) {
    src.split("\n").forEach((line, i) => {
      if (!line.includes("holding_days")) { return; }
      scanned += 1;
      if (line.includes("default_holding_days()")) { return; }
      // 声明处的注释不计入违规（只在真代码上要求）
      if (line.trim().startsWith("//!") || line.trim().startsWith("//")) { return; }
      violations.push(`${name}:${i + 1} ${line.trim()}`);
    });
  }
  return { violations, scanned };
}

/** 门 b：`period.factor()` 只能出现在名字含 fallback 的函数里。 */
function checkFactorOnlyInFallback(src, fileLabel) {
  const lines = src.split("\n");
  const fnStart = /^\s*(?:pub )?(?:async )?fn ([A-Za-z0-9_]+)/;
  let current = "<file top>";
  const violations = [];
  let scanned = 0;
  lines.forEach((line, i) => {
    const m = line.match(fnStart);
    if (m) { current = m[1]; }
    if (!line.includes(".factor()")) { return; }
    scanned += 1;
    if (!current.includes("fallback")) {
      violations.push(`${fileLabel}:${i + 1} 在非 fallback 函数 \`${current}()\` 里乘 factor()`);
    }
  });
  return { violations, scanned };
}

/** 门 c：三处拍脑袋校准数必须归零。 */
function checkAdHocCalibration(src, fileLabel) {
  const patterns = [
    [/, "neutral"\)/, `weighted_signal_calibration(..., "neutral") —— 市场状态被写死`],
    [/posterior_win_rate \/ 0\.50/, "胜率基准写死 0.50（应为全档合并基准 p_pool）"],
    [/=> 0\.85,\s*\/\/\s*超短/, "反身性折扣 0.85 只对超短硬编码，无推导"],
  ];
  const violations = [];
  let scanned = 0;
  for (const [re, why] of patterns) {
    if (re.test(src)) {
      const line = src.split("\n").findIndex((l) => re.test(l)) + 1;
      violations.push(`${fileLabel}:${line} ${why}`);
    }
    scanned += 1;
  }
  return { violations, scanned };
}

/** 门 d：前端档位序必须与后端 `Period::ALL` 逐项同序。 */
function checkTierOrderSingleSource() {
  const holding = read(HOLDING);
  const allMatch = holding.match(/pub const ALL: \[Period; 4\] = \[([^\]]+)\]/);
  if (!allMatch) { return { violations: [`${HOLDING} 解析不到 Period::ALL`], scanned: 0 }; }
  const backend = [...allMatch[1].matchAll(/Period::([A-Za-z]+)/g)].map((m) =>
    m[1].replace(/^UltraShort$/, "ultra_short").toLowerCase(),
  );
  const order = (rel, re) => {
    const m = read(rel).match(re);
    return m ? [...m[1].matchAll(/"([a-z_]+)"/g)].map((x) => x[1]) : null;
  };
  const frontends = [
    [PANEL, /const PERIOD_ORDER: PeriodKey\[\] = \[([^\]]+)\]/],
    [MATRIX, /const PERIOD_KEYS = \[([^\]]+)\] as const/],
  ];
  const violations = [];
  for (const [rel, re] of frontends) {
    const got = order(rel, re);
    if (!got) {
      violations.push(`${rel} 解析不到档位序常量（判据失效，不是「没问题」）`);
      continue;
    }
    if (got.join(",") !== backend.join(",")) {
      violations.push(`${rel} 档位序 ${got.join(",")} ≠ 后端 Period::ALL ${backend.join(",")}`);
    }
  }
  return { violations, scanned: frontends.length };
}

/** 门 e：策略×档位矩阵必须 24 格齐全（出票 or 显式不成立理由）。 */
function checkStylePeriodMatrix(src, fileLabel) {
  const rows = [...src.matchAll(/\(\s*"([a-z_]+)",\s*"([a-z_]+)",\s*(Some\("([a-z_]+)"\)|None)\s*\)/g)];
  const violations = [];
  for (const t of ["ultra_short", "short", "mid", "long"]) {
    for (const s of ["trend", "value", "capital", "reversion", "watchlist", "serenity"]) {
      if (!rows.some((m) => m[1] === s && m[2] === t)) {
        violations.push(`${fileLabel} 矩阵缺格 ${s}×${t}（缺格 ≠ 不做，必须逐格点名）`);
      }
    }
  }
  return { violations, scanned: rows.length };
}

/** 门 f：策略不得自己写死 "daily" 取数（尺度必须由 scale 层按档位解析）。 */
function checkScaleSingleSource(files) {
  const violations = [];
  let scanned = 0;
  for (const { name, src } of files) {
    src.split("\n").forEach((line, i) => {
      const usesScale = line.includes("scale::fetch(") || line.includes("recommender::scale::fetch(");
      const usesRaw = line.includes("get_klines(");
      if (!usesScale && !usesRaw) { return; }
      scanned += 1;
      if (usesRaw && /"daily"/.test(line)) {
        violations.push(`${name}:${i + 1} 写死 "daily" 取数 ⇒ 档位尺度未收编`);
      }
    });
  }
  return { violations, scanned };
}

/** --selftest：把每门的「修复前真实形态」当夹具喂进判据，必须红。 */
function selftest() {
  const cases = [
    {
      name: "a 正控（默认天数）应绿",
      got: checkHoldingDaysSingleSource([
        { name: "x.rs", src: "            holding_days: self.period.default_holding_days()," },
      ]).violations.length,
      want: 0,
    },
    {
      name: "a 负控（自带 7 天表）应红",
      got: checkHoldingDaysSingleSource([
        { name: "x.rs", src: "        let (a, b, holding_days) = (1.0, 2.0, 7);" },
      ]).violations.length,
      want: 1,
    },
    {
      name: "b 正控（fallback 函数内）应绿",
      got: checkFactorOnlyInFallback(
        "pub fn calc_position_fallback(base: f64, period: Period) -> f64 {\n    base * period.factor()\n}",
        "x",
      ).violations.length,
      want: 0,
    },
    {
      name: "b 负控（主路径乘 factor）应红",
      got: checkFactorOnlyInFallback(
        "pub fn calc_position_with_consistency(base: f64, period: Period) -> f64 {\n    base * period.factor()\n}",
        "x",
      ).violations.length,
      want: 1,
    },
    {
      name: "c 负控（三处硬编码都在）应红",
      got: checkAdHocCalibration(
        'calibration(&id, "neutral");\n((posterior_win_rate / 0.50) - 0.20)\nUltraShort => 0.85, // 超短线博弈性强\n',
        "x",
      ).violations.length,
      want: 3,
    },
    {
      name: "e 负控（缺一格）应红",
      got: checkStylePeriodMatrix('( "trend", "short", None),', "x").violations.length,
      want: 23,
    },
    {
      name: "f 负控（写死 daily）应红",
      got: checkScaleSingleSource([
        { name: "x.rs", src: '        let k = client.get_klines(code, "daily", 60).await;' },
      ]).violations.length,
      want: 1,
    },
    {
      name: "f 正控（走 scale 层）应绿",
      got: checkScaleSingleSource([
        { name: "x.rs", src: "        let k = scale::fetch_klines(client, code, self.period).await;" },
      ]).violations.length,
      want: 0,
    },
  ];
  let bad = 0;
  for (const c of cases) {
    const ok = c.got === c.want;
    if (!ok) { bad += 1; }
    console.log(`${ok ? "PASS" : "FAIL"} ${c.name}（got=${c.got} want=${c.want}）`);
  }
  // 门 d 的负控要求真文件确实可解析（解析不到 = 判据失效）
  const d = checkTierOrderSingleSource();
  if (d.scanned === 0) {
    bad += 1;
    console.log("FAIL d 扫描面为 0（判据失效）");
  } else {
    console.log(`PASS d 扫描面=${d.scanned}`);
  }
  process.exit(bad === 0 ? 0 : 1);
}

function main() {
  const strategyFiles = fs
    .readdirSync(path.join(ROOT, STRATEGIES_DIR))
    .filter((f) => f.endsWith(".rs"))
    .map((f) => ({ name: `${STRATEGIES_DIR}/${f}`, src: read(`${STRATEGIES_DIR}/${f}`) }));

  const gates = [
    ["a 持有天数唯一来源", checkHoldingDaysSingleSource(strategyFiles)],
    ["b 仓位 factor 只在 fallback", checkFactorOnlyInFallback(read(SCORING), SCORING)],
    ["c 校准数不得拍脑袋", checkAdHocCalibration(read(RECO_MOD), RECO_MOD)],
    ["d 档位序单源（前端 vs Period::ALL）", checkTierOrderSingleSource()],
    [
      "e 风格×档位矩阵完备（24 格）",
      fs.existsSync(path.join(ROOT, MATRIX_SRC))
        ? checkStylePeriodMatrix(read(MATRIX_SRC), MATRIX_SRC)
        : { violations: [`${MATRIX_SRC} 不存在 ⇒ 无「哪个风格在哪档成立」的契约表`], scanned: 0 },
    ],
    ["f 取数尺度单源（禁写死 daily）", checkScaleSingleSource(strategyFiles)],
  ];

  let red = 0;
  for (const [label, res] of gates) {
    if (res.scanned === 0) {
      console.log(`BROKEN ${label}：扫描面为 0 ⇒ 判据失效（按假绿处理）`);
      red += 1;
      continue;
    }
    if (res.violations.length > 0) {
      console.log(`RED ${label}（扫描面 ${res.scanned}）：`);
      for (const v of res.violations) { console.log(`  - ${v}`); }
      red += res.violations.length;
    } else {
      console.log(`OK  ${label}（扫描面 ${res.scanned}）`);
    }
  }
  if (red > 0) {
    console.log(`\n结论: ${red} 处违规/失效`);
    process.exit(1);
  }
  console.log("\n结论: 荐股四档口径一致");
}

if (process.argv.includes("--selftest")) { selftest(); } else { main(); }
