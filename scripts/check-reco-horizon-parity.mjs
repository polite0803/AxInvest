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

/**
 * 尺度豁免理由码：**必须显式登记**。判据认 `reco-scale-exempt: <code>`（同行或紧邻上一行），
 * 未登记的码按「未豁免」处理并单列违规 —— 豁免是加一条更严的登记，不是放宽判据。
 *
 * `calendar-window`：窗口本意是日历量（如「近 3 月涨幅」），日线就是它的正确尺度，
 * 换成档位尺度反而语义漂移（20 个交易日到季线 = 20 季 ≈ 10 年）。
 */
const SCALE_EXEMPT_REASONS = new Set(["calendar-window"]);

/** 门 f：策略不得自己写死 "daily" 取数（尺度必须由 scale 层按档位解析）。
 *
 * 判据覆盖 `get_klines` 的**全部同族方法名**。旧判据是 `line.includes("get_klines(")`，
 * 于是 `get_klines_with_adj(code, "daily", 252)` 整条绕过 —— 实测修复前「扫描面 4、结论 OK」
 * 的假绿（`strategies/serenity.rs:124` 是全目录下唯一写死日线的取数，门看不见它）。
 */
function checkScaleSingleSource(files) {
  const violations = [];
  let scanned = 0;
  let exempted = 0;
  for (const { name, src } of files) {
    const lines = src.split("\n");
    lines.forEach((line, i) => {
      const usesScale = line.includes("scale::fetch(") || line.includes("recommender::scale::fetch(");
      const usesRaw = /\.get_klines\w*\(/.test(line);
      if (!usesScale && !usesRaw) { return; }
      scanned += 1;
      if (!usesRaw || !/"daily"/.test(line)) { return; }
      const exemptCode = (l) => (l.match(/reco-scale-exempt:\s*([a-z0-9-]+)/) ?? [])[1];
      const code = exemptCode(line) ?? exemptCode(lines[i - 1] ?? "");
      if (code === undefined) {
        violations.push(`${name}:${i + 1} 写死 "daily" 取数 ⇒ 档位尺度未收编`);
      } else if (!SCALE_EXEMPT_REASONS.has(code)) {
        violations.push(`${name}:${i + 1} 豁免理由码 "${code}" 未登记 ⇒ 视为未豁免`);
      } else {
        exempted += 1;
      }
    });
  }
  return { violations, scanned, exempted };
}

const LOCALE_LANGS = ["ar", "de", "en-US", "es", "fr", "hi", "ja", "ko", "ru", "zh-CN", "zh-TW"];
const LOCALES_DIR = "src/i18n/locales";

/**
 * 门 g：矩阵的每个理由码都必须在**全部 11 语言**里有文案。
 *
 * 为什么锁这条：`style_matrix` 的立项理由就是「不成立的格必须带机器可读的理由码」，
 * 而理由码只有后端有、前端没翻译时，UI 依旧只能退回原始码或空白 —— 契约写对了，
 * 呈现层仍然是歧义。缺席声明的**最后一公里**是翻译，本门把两者钉在一起。
 */
function checkAbsenceReasonTranslated(src) {
  const violations = [];
  let scanned = 0;
  const absence = new Set();
  for (const m of src.matchAll(/Some\("([a-z0-9_]+)"\)/g)) { absence.add(m[1]); }
  // 兜底码：矩阵漏格时后端也输出它，同样必须有文案（否则「漏格」这一最危险的状态没解释）
  absence.add("cell_not_in_matrix");
  const misfit = new Set();
  // 锚在 `= [ … ];` 上：类型标注 `[(&str, &str, &str); 1]` 里也含 `[`/`;`，
  // 只找第一个 `[` 会把**类型**当成数组来解析 ⇒ 错配码集合恒空（门静默失效）。
  const mis = src.match(/MISFIT_DECLARATIONS[\s\S]*?=\s*\[([\s\S]*?)\]\s*;/);
  if (mis) {
    for (const m of mis[1].matchAll(/\(\s*"[a-z_]+",\s*"[a-z_]+",\s*"([a-z0-9_]+)"\s*\)/g)) {
      misfit.add(m[1]);
    }
  }
  for (const lang of LOCALE_LANGS) {
    let json;
    try {
      json = JSON.parse(read(`${LOCALES_DIR}/${lang}.json`));
    } catch {
      violations.push(`${lang}.json 解析失败 ⇒ 无法核对理由码`);
      continue;
    }
    const bt = (json.stockAnalysis ?? {}).backtest ?? {};
    for (const code of absence) {
      scanned += 1;
      const v = ((bt.matrixAbsence ?? {})[code]);
      if (typeof v !== "string" || v.length === 0) {
        violations.push(`${lang}: 理由码 ${code} 缺 matrixAbsence 翻译 ⇒ 界面退回原始码`);
      }
    }
    for (const code of misfit) {
      scanned += 1;
      const v = ((bt.matrixMisfit ?? {})[code]);
      if (typeof v !== "string" || v.length === 0) {
        violations.push(`${lang}: 错配码 ${code} 缺 matrixMisfit 翻译`);
      }
    }
  }
  return { violations, scanned };
}

const SERENITY_WORKFLOW = "src-tauri/src/commands/stock_workflow/serenity.rs";
const SERENITY_STRATEGY = `${STRATEGIES_DIR}/serenity.rs`;

/**
 * 门 h：趋势智选**两条链**的风控口径必须同源（`PLAN-serenity-horizon-adaptation.md` S1/S2）。
 *
 * 修复前的真实形态（本门对它必须报红）：
 *  - 工作流链 `commands/stock_workflow/serenity.rs` 自己按 `price * serenity_stop_mult` 出止损，
 *    而策略链经 `recommender/mod.rs` 的 `k1·σ_daily·√h` + 风险预算 ⇒ **同一张 reco_picks、
 *    同一个趋势智选历史列表**里并存两套口径，且不声明哪套不是波动率口径；
 *  - 落库缺 `stopSource/positionSource/priorSource` ⇒ 前端无从降级出句；
 *  - 策略链的近 12 月涨幅过滤直接取 `klines.first()` ⇒ 次新股拿「上市以来涨幅」比 12 月阈值；
 *    K 线不足 / 取数失败时**静不过滤**，卡片照样显示「确定性财务验证通过」。
 */
function checkSerenityRiskParity(workflowSrc, strategySrc) {
  const violations = [];
  let scanned = 0;
  const require = (src, needle, why, label) => {
    scanned += 1;
    if (!src.includes(needle)) { violations.push(`${label}: 缺 ${needle} ⇒ ${why}`); }
  };
  const forbid = (src, needle, why, label) => {
    scanned += 1;
    if (src.includes(needle)) { violations.push(`${label}: 仍含 ${needle} ⇒ ${why}`); }
  };
  for (const fn of ["daily_closes", "stop_pct", "target_pct", "risk_budget_position", "cost_drag_factor"]) {
    require(workflowSrc, `recommender::risk::${fn}(`, "工作流链没接荐股链同一套波动率风控（两套口径）", "工作流链");
  }
  forbid(workflowSrc, "price * serenity_stop_mult", "固定乘数止损又回来了", "工作流链");
  forbid(workflowSrc, "price * serenity_target_mult", "固定乘数目标又回来了", "工作流链");
  for (const key of ["stopSource", "positionSource", "priorSource"]) {
    require(workflowSrc, `"${key}"`, "落库缺来源键 ⇒ 前端无法声明该 pick 不是波动率口径", "工作流链");
  }
  for (const key of ["reco_stop_vol_mult", "reco_target_vol_mult", "reco_risk_budget_pct", "reco_round_trip_cost_pct"]) {
    require(workflowSrc, `"${key}"`, "风控参数键与荐股链不同源 ⇒ 同一票两链算出不同的 k1/k2/R", "工作流链");
  }
  require(strategySrc, "reco-scale-exempt: calendar-window", "日历窗口取数未登记豁免 ⇒ 门 f 应当报红", "策略链");
  require(strategySrc, "bars < 252", "近12月过滤缺长度守卫 ⇒ 次新股按「上市以来涨幅」比 12 月阈值", "策略链");
  require(strategySrc, "gain_filter_skips", "过滤未执行没有状态记录 ⇒ 静默放行", "策略链");
  require(strategySrc, "涨幅过滤未生效", "未生效时未成句声明 ⇒ 卡片冒充「验证通过」", "策略链");
  return { violations, scanned };
}

/** 读取 strategies/ 全部 .rs（门 a/f 的扫描对象）。selftest 与 main 共用同一份，避免两建面不同。 */
function readStrategyFiles() {
  return fs
    .readdirSync(path.join(ROOT, STRATEGIES_DIR))
    .filter((f) => f.endsWith(".rs"))
    .map((f) => ({ name: `${STRATEGIES_DIR}/${f}`, src: read(`${STRATEGIES_DIR}/${f}`) }));
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
      name: "f 负控（写死 daily，get_klines）应红",
      got: checkScaleSingleSource([
        { name: "x.rs", src: '        let k = client.get_klines(code, "daily", 60).await;' },
      ]).violations.length,
      want: 1,
    },
    {
      // 这条就是修复前门**看不见**的真实形态：同族方法名 `_with_adj` + 写死 "daily"。
      // 旧判据 `includes("get_klines(")` 在此返回 false ⇒ 假绿。
      name: "f 负控（同族方法名 get_klines_with_adj 写死 daily，serenity 真实形态）应红",
      got: checkScaleSingleSource([
        {
          name: "x.rs",
          src: '        if let Ok(klines) = client.get_klines_with_adj(code, "daily", 252, None).await {',
        },
      ]).violations.length,
      want: 1,
    },
    {
      name: "f 负控（豁免码未登记）应红",
      got: checkScaleSingleSource([
        {
          name: "x.rs",
          src: '        // reco-scale-exempt: whatever\n        let k = client.get_klines(code, "daily", 60).await;',
        },
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
    {
      name: "f 正控（登记过的日历窗口豁免）应绿",
      got: checkScaleSingleSource([
        {
          name: "x.rs",
          src: '        // reco-scale-exempt: calendar-window\n        let k = client.get_klines_with_adj(code, "daily", 252, None).await;',
        },
      ]).violations.length,
      want: 0,
    },
    {
      // 新加理由码却忘了补 11 语言翻译 —— 正是本轮要在动手前就被拦住的形态
      name: "g 负控（新不成立理由码没翻译）应红",
      got: checkAbsenceReasonTranslated('("trend", "long", Some("brand_new_absence_code")),')
        .violations.length,
      want: 11,
    },
    {
      name: "g 负控（新错配码没翻译）应红",
      got: checkAbsenceReasonTranslated(
        'pub const MISFIT_DECLARATIONS: [(&str, &str, &str); 1] =\n    [("value", "ultra_short", "brand_new_misfit_code")];',
      ).violations.length,
      want: 11,
    },
    {
      // 修复前的工作流链真实形态：固定乘数出止损/目标 + 落库无来源键（策略链用现盘文件，0 违规）
      name: "h 负控（工作流链仍用固定乘数）应红",
      got: checkSerenityRiskParity(
        '        let (entry_low, entry_high, stop_loss, target_price) = if price > 0.0 {\n'
          + "            (price * (1.0 - serenity_entry_range), price * serenity_stop_mult, price * serenity_target_mult)\n"
          + '        };\n                        "positionPct": 5.0,\n                        "riskNotes": [],\n',
        read(SERENITY_STRATEGY),
      ).violations.length,
      want: 14,
    },
    {
      name: "h 负控（策略链静默放行涨幅过滤）应红",
      got: checkSerenityRiskParity(
        read(SERENITY_WORKFLOW),
        '        if let Ok(klines) = client.get_klines_with_adj(code, "daily", 252, None).await {\n'
          + "            if let Some(first) = klines.first() {\n                let gain = (latest - first.close) / first.close;\n            }\n        }\n",
      ).violations.length,
      want: 4,
    },
    {
      name: "h 正控（当前两链形态）应绿",
      got: checkSerenityRiskParity(read(SERENITY_WORKFLOW), read(SERENITY_STRATEGY)).violations.length,
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
  // 门 f 的**真文件扫描面**断言：夹具绿不代表门看得见生产代码。
  // 旧判据对 `.get_klines_with_adj(` 恒不命中 ⇒ 假绿，正是靠这两条断言才暴露：
  //  ① scanned 必须 ≥5（五策略的 scale::fetch + serenity 的日历窗口取数）；
  //  ② serenity 那行必须落在「已豁免」桶里（exempted ≥1），而不是根本没被扫到。
  const f = checkScaleSingleSource(readStrategyFiles());
  if (f.scanned < 5) {
    bad += 1;
    console.log(`FAIL f 真文件扫描面=${f.scanned}（<5 ⇒ 同族取数方法名又漏了）`);
  } else {
    console.log(`PASS f 真文件扫描面=${f.scanned} 已豁免=${f.exempted}`);
  }
  if (f.exempted < 1) {
    bad += 1;
    console.log("FAIL f 没有任何日历窗口豁免登记 ⇒ serenity 的日线取数不在门视野内");
  }
  // 门 g 的**真文件判定面**断言：3 个不成立码 + 漏格兜底码 + 1 个错配码 = 5 码 × 11 语言。
  // 面不足 ⇒ 理由码解析退化（历史上 `[^;]*` 会先吃到类型标注 `[(&str, &str, &str); 1]`，
  // 错配集合恒空，这一支就永远不会红）。
  if (fs.existsSync(path.join(ROOT, MATRIX_SRC))) {
    const g = checkAbsenceReasonTranslated(read(MATRIX_SRC));
    if (g.scanned < 55) {
      bad += 1;
      console.log(`FAIL g 真文件判定面=${g.scanned}（<55 ⇒ 理由码/错配码解析退化）`);
    } else {
      console.log(`PASS g 真文件判定面=${g.scanned} 违规=${g.violations.length}`);
    }
    if (g.violations.length > 0) {
      bad += 1;
      console.log("FAIL g 真文件存在未翻译的理由码");
    }
  }
  process.exit(bad === 0 ? 0 : 1);
}

function main() {
  const strategyFiles = readStrategyFiles();

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
    [
      "g 缺席/错配理由码 11 语言全译",
      fs.existsSync(path.join(ROOT, MATRIX_SRC))
        ? checkAbsenceReasonTranslated(read(MATRIX_SRC))
        : { violations: [], scanned: 0 },
    ],
    [
      "h 趋势智选两链风控同源",
      fs.existsSync(path.join(ROOT, SERENITY_WORKFLOW)) && fs.existsSync(path.join(ROOT, SERENITY_STRATEGY))
        ? checkSerenityRiskParity(read(SERENITY_WORKFLOW), read(SERENITY_STRATEGY))
        : { violations: [`${SERENITY_WORKFLOW} 或 策略链文件不存在 ⇒ 无从比对`], scanned: 0 },
    ],
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
