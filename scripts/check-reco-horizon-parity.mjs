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
 * 取 `const <name>… = [ … ];` 的数组体。
 * 必须锚在 `= [` 上：直接找第一个 `[` 会先吃到**类型标注**（`[Cell; 24]`、
 * `[(&str, &str, &str); 1]`）⇒ 解析到的是类型而不是数据，集合恒空而门照样绿。
 * 名字前必须带**词边界**：`RENAMED_MATRIX` 里含 `MATRIX`，无边界时改名后的常量
 * 仍被当成原契约解析 ⇒ 「判据失效」这一支永远不会红（本仓的负控夹具实测把它撞了出来，
 * 与门 f 的 `get_klines(` 漏检同族形态）。
 */
function arrayBody(src, name) {
  const m = src.match(new RegExp("(?:^|[^A-Za-z0-9_])" + name + "[^=]*=\\s*\\[([\\s\\S]*?)\\]\\s*;", "m"));
  return m ? m[1] : "";
}

/**
 * 门 g：矩阵的每个理由码都必须在**全部 11 语言**里有文案。
 *
 * 为什么锁这条：`style_matrix` 的立项理由就是「不成立的格必须带机器可读的理由码」，
 * 而理由码只有后端有、前端没翻译时，UI 依旧只能退回原始码或空白 —— 契约写对了，
 * 呈现层仍然是歧义。缺席声明的**最后一公里**是翻译，本门把两者钉在一起。
 *
 * 扫描面只取 **MATRIX / MISFIT 声明体**：全文匹配 `Some("…")` 会把测试正文里的断言
 * （例：`assert_eq!(absence_reason(…), Some("cell_is_active"))`）当成契约码，
 * 门于是去要一个根本不存在的翻译 —— 红得毫无道理，且下次没人再看这条门。
 */
function checkAbsenceReasonTranslated(src) {
  const violations = [];
  let scanned = 0;
  const absence = new Set();
  const mat = arrayBody(src, "MATRIX");
  if (mat.length === 0) {
    violations.push(`${MATRIX_SRC}: 未解析到 MATRIX 声明体 ⇒ 判据失效`);
  }
  for (const m of mat.matchAll(/Some\("([a-z0-9_]+)"\)/g)) { absence.add(m[1]); }
  // 兜底码：矩阵漏格时后端也输出它，同样必须有文案（否则「漏格」这一最危险的状态没解释）
  absence.add("cell_not_in_matrix");
  const misfit = new Set();
  const mis = arrayBody(src, "MISFIT_DECLARATIONS");
  for (const m of mis.matchAll(/\(\s*"[a-z_]+",\s*"[a-z_]+",\s*"([a-z0-9_]+)"\s*\)/g)) {
    misfit.add(m[1]);
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
const RECO_RISK = "src-tauri/crates/analysis-engine/src/recommender/risk.rs";

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
/** 取 zh-CN 的 `serenityPanel.tierScopeHint` —— 档位口径的**声明面**（对照代码的产出面）。 */
function serenityTierScopeHint() {
  try {
    const j = JSON.parse(read(`${LOCALES_DIR}/zh-CN.json`));
    return ((j.serenityPanel ?? {}).tierScopeHint) ?? "";
  } catch {
    return "";
  }
}

/** 剥掉**整行注释**后再做契约比对。
 *
 * 实证（本轮）：反向锁 `price * serenity_stop_mult` 命中了我自己写的说明注释
 * 「旧形态：本链按 `price * serenity_stop_mult` 出固定百分比止损」⇒ 门对一段
 * 描述历史的散文报红。反向锁若能被注释触红，后果不是修代码而是删注释 —— 判据失效。
 * 只剥整行注释、不碰行尾注释与字符串：门 f 的豁免标记 `reco-scale-exempt:` 恰恰写在
 * 注释里，那边**必须**保留注释，所以本函数只在门 h 内使用。
 */
function stripFullLineComments(src) {
  return src
    .split("\n")
    .filter((l) => !l.trim().startsWith("//"))
    .join("\n");
}

/**
 * 门 i：面板「档位口径」文案必须与代码的档位集合一致。
 *
 * 为什么单独锁：Q2 裁定工作流链从恒 mid 改成 mid + long 各一行 —— 代码改了而
 * `serenityPanel.tierScopeHint` 还写着「仅服务中线 / 落库 period 恒为 mid」，就是
 * **UI 与产出互相矛盾的第二份真相**（用户按文案理解，数据却两档）。
 * 同时锁住「不得把权威天数抄进文案」：28/90 的唯一来源是 `Period::default_holding_days`，
 * 抄进 i18n 就成了改了权威表也不会跟着变的死数字（同 `check-horizon-weight-parity.mjs` ④）。
 */
function checkTierScopeHint(hint) {
  const violations = [];
  let scanned = 0;
  scanned += 1;
  if (!hint) { return { violations: ["tierScopeHint 为空 ⇒ 无从核对档位口径"], scanned }; }
  for (const [pat, why] of [
    [/恒为\s*mid/, "文案仍声明「落库恒为 mid」⇒ 与代码的 mid+long 双档矛盾"],
    [/仅服务中线|只服务中\/长线/, "文案仍声明单档 ⇒ 与代码的 mid+long 双档矛盾"],
  ]) {
    scanned += 1;
    if (pat.test(hint)) { violations.push(why); }
  }
  scanned += 1;
  if (!/长线/.test(hint)) { violations.push("文案未声明服务长线 ⇒ Q2 的双档在 UI 上不可见"); }
  scanned += 1;
  if (/\b(28|90)\s*天/.test(hint)) {
    violations.push("文案抄了权威天数（28/90）⇒ 与 `Period::default_holding_days` 两处真相");
  }
  return { violations, scanned };
}

function checkSerenityRiskParity(workflowSrcRaw, strategySrcRaw, riskSrc) {
  const workflowSrc = stripFullLineComments(workflowSrcRaw);
  const strategySrc = stripFullLineComments(strategySrcRaw);
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
  require(strategySrc, "bars < 252", "近12月过滤缺长度守卫 ⇒ 次新股按「上市以来涨幅」比 12 月阈值", "策略链");
  require(strategySrc, "gain_filter_skips", "过滤未执行没有状态记录 ⇒ 静默放行", "策略链");
  require(strategySrc, "涨幅过滤未生效", "未生效时未成句声明 ⇒ 卡片冒充「验证通过」", "策略链");
  // Q1 裁定 B：建仓带 = 该档止损距离的一半，且**两链调同一个函数**（各自写一遍公式就是两套口径）
  require(workflowSrc, "risk::entry_band_pct(", "工作流链建仓带没走 Q1=B 的唯一推导", "工作流链");
  require(strategySrc, "risk::entry_band_pct(", "策略链建仓带没走 Q1=B 的唯一推导", "策略链");
  forbid(workflowSrc, "price * (1.0 - serenity_entry_range)", "固定 ±range 建仓带又回来了", "工作流链");
  forbid(strategySrc, "price * (1.0 - entry_range)", "固定 ±range 建仓带又回来了", "策略链");
  require(workflowSrc, `"entrySource"`, "落库缺建仓带来源键 ⇒ 退化时无从声明", "工作流链");
  // Q2 裁定：工作流链按 serenity 的出票档（mid + long）各落一行，与 style_matrix 的 serenity 行一致
  require(workflowSrc, "for tier in serenity_tiers", "工作流链仍只出单档 ⇒ Q2 的双档没落地", "工作流链");
  require(
    workflowSrc,
    "Period::Mid, axagent_harness::Period::Long",
    "serenity 档位集合没读进落库循环 ⇒ 与 style_matrix 的 serenity 行脱钩",
    "工作流链",
  );
  require(
    workflowSrc,
    "{tier_period}",
    "reco_picks id 不带档位后缀 ⇒ mid/long 两行主键相撞，后者静默丢",
    "工作流链",
  );
  // 建仓带公式的**唯一性**：定义一处、两链调用，链文件里不得再内联 `/ 2.0`
  require(riskSrc, "pub fn entry_band_pct(", "建仓带没有唯一实现 ⇒ 两链必然各写一套", "risk 层");
  forbid(workflowSrc, "stop_pct / 2.0", "工作流链内联推导建仓带（应调 risk::entry_band_pct）", "工作流链");
  forbid(strategySrc, "stop_pct / 2.0", "策略链内联推导建仓带（应调 risk::entry_band_pct）", "策略链");
  return { violations, scanned };
}

/**
 * 门 j：趋势智选**候选卡片**必须把档位与风控口径呈现出来（用户裁定 A）。
 *
 * 为什么单独锁：Q2 把产出层改成逐档两行后，呈现层若不跟着改，「四周期适配」就只存在于
 * 数据库与历史弹窗里 —— 面板看上去和改之前一模一样（本轮实测就是这状态）。
 * 三件事钉死：① 卡上有档徽标（且走全仓唯一档名键族，不许再造第三套）；
 * ② 没有 `?? 20` 这种把「未标档」压成读数 20 的兜底（与 `Period::Mid` 的 28 天矛盾）；
 * ③ 未标档必须走独立成句的 `timeBasisNoTier`，不是留空也不是猜档。
 */
function checkSerenityCardTierDisplay(cardSrc) {
  const violations = [];
  let scanned = 0;
  const require = (needle, why) => {
    scanned += 1;
    if (!cardSrc.includes(needle)) { violations.push(`趋势智选卡片: 缺 ${needle} ⇒ ${why}`); }
  };
  const forbid = (needle, why) => {
    scanned += 1;
    if (cardSrc.includes(needle)) { violations.push(`趋势智选卡片: 仍含 ${needle} ⇒ ${why}`); }
  };
  require("horizonSuffix(", "档名没走唯一键族 stockAnalysis.timeHorizon*（会造出第三套档名）");
  require('data-testid="serenity-tier"', "卡上没有档位徽标 ⇒ Q2 的逐档产出在面板不可见");
  require("timeBasisNoTier", "未标档没有独立成句 ⇒ 会被读成「有窗口但没显示」");
  require("holdingDays === undefined", "时间基线不分「有天数/无天数」两支 ⇒ 未标档也照渲一个窗口");
  forbid("?? 20", "把「无天数」兜底成 20 天 ⇒ 与 mid 权威 28 天矛盾（落库侧已修，呈现层不得留）");
  return { violations, scanned };
}

const SERENITY_CARD = "src/components/stock-analysis/SerenityCandidateCard.tsx";

// ── 窗口涨幅达标漏检核查（`PLAN-mover-recall-attribution.md`）──
const MOVER_ENGINE = "src-tauri/crates/analysis-engine/src/mover_recall.rs";
const MOVER_CMD = "src-tauri/src/commands/mover_recall.rs";
const MOVER_PANEL = "src/components/stock-analysis/MoverRecallPanel.tsx";
const SEED_VARS = "src-tauri/src/commands/stock_analysis_setup/seed_variables.rs";
const MOVER_TIERS = ["ultra_short", "short", "mid", "long"];

/** 剥掉**整行注释**（行首为 `//`、块注释起止行、星号续行）——口径边界句写在注释里是合法的，
 *  判据只锁**呈现面**；不剥注释会让「我没写涨停」的说明句自己把门撞红。 */
function stripCommentLines(src) {
  return src
    .split("\n")
    .filter((l) => {
      const s = l.trim();
      return !(s.startsWith("//") || s.startsWith("/*") || s.startsWith("*") || s.startsWith("*/"));
    })
    .join("\n");
}

/**
 * 门 k：窗口涨幅达标的四档阈值 + 窗口天数必须单源。
 *
 * 修复前的真实形态（本门对它必须报红）：
 *  - 缺省阈值同时写在引擎常量表与模板变量种子里 —— 两处各写一份 ⇒ 改了变量不改代码，
 *    「用户调过的阈值」与「代码兜底值」静默漂移，而任何编译器/单测都不会报；
 *  - 窗口天数若就地写死（如 `window_days: 5`）⇒ 与 `Period::default_holding_days`
 *    形成第二套档位尺度（同门 a / 门 f 的同族形态）。
 */
function checkMoverThresholdSingleSource(engineSrc, seedSrc) {
  const violations = [];
  let scanned = 0;
  const body = arrayBody(engineSrc, "DEFAULT_GAIN_THRESHOLDS");
  if (body.length === 0) {
    return { violations: [`${MOVER_ENGINE}: 未解析到 DEFAULT_GAIN_THRESHOLDS ⇒ 判据失效`], scanned };
  }
  const rows = [...body.matchAll(/\("([a-z0-9_]+)",\s*([0-9.]+)\)/g)].map((m) => [m[1], m[2]]);
  for (const tier of MOVER_TIERS) {
    scanned += 1;
    const row = rows.find(([name]) => name === `mover_gain_${tier}`);
    if (!row) {
      violations.push(`${MOVER_ENGINE}: 缺档位 ${tier} 的出厂阈值（四档必须逐档登记）`);
      continue;
    }
    // 种子变量的 value 与 name 之间可能夹着 var_type/description 行 —— 限窗内匹配
    const seed = seedSrc.match(
      new RegExp(`name:\\s*"mover_gain_${tier}"[\\s\\S]{0,400}?value:\\s*serde_json::json!\\(([0-9.]+)\\)`),
    );
    scanned += 1;
    if (!seed) {
      violations.push(`${SEED_VARS}: 缺模板变量 mover_gain_${tier} ⇒ 该档阈值无用户可调入口`);
    } else if (Number(seed[1]) !== Number(row[1])) {
      // 按**数值**比而不是按字面量：`json!(10)` 与 `10.0` 是同一个阈值，
      // 按串比会把十进制定点的写法差异报成「两处真相」——判据失焦。
      violations.push(
        `阈值漂移：mover_gain_${tier} 引擎出厂 ${row[1]} ≠ 种子默认 ${seed[1]}（两处真相）`,
      );
    }
  }
  scanned += 1;
  if (!engineSrc.includes("default_holding_days()")) {
    violations.push(`${MOVER_ENGINE}: 未取 default_holding_days ⇒ 窗口天数成了第二套档位尺度`);
  }
  for (const m of engineSrc.matchAll(/window_days:\s*([0-9]+)\b/g)) {
    scanned += 1;
    violations.push(`${MOVER_ENGINE}: window_days 写死字面量 ${m[1]} ⇒ 天数必须取 default_holding_days()`);
  }
  return { violations, scanned };
}

/**
 * 从种子文件里只取 `mover_gain_*` 四个变量的 description 文案。
 *
 * 判据窗口必须**窄到这几条**：同一文件里另有「涨停潜力评分」等其它功能的变量
 * （`limit_pct_main` 一族，天生就该写「涨停」），整文件扫描会红得毫无道理；
 * 源码里的**注释**同理（口径边界句正写在注释里），故只取字符串字面量。
 */
function moverVarDescriptions(seedSrc) {
  const out = [];
  for (const tier of MOVER_TIERS) {
    const m = seedSrc.match(
      new RegExp(`name:\\s*"mover_gain_${tier}"[\\s\\S]{0,400}?description:\\s*Some\\("([^"]*)"`),
    );
    if (m) { out.push([tier, m[1]]); }
  }
  return out;
}

/**
 * 门 l：口径边界 —— 判据是**绝对涨幅**，用户可见文案一律不得出现「涨停」字样。
 *
 * 名大于内容即为歧义：若面板/文案写「涨停」，用户会以为事件判据含板块涨停语义
 * （主板 10% / 创业科 20% / 北交 30%），而实际判据是不分板块的绝对涨幅。
 * 扫描面 = 引擎 + 命令 + 面板（剥整行注释）+ `mover_gain_*` 变量描述 + 11 语言的
 * `stockAnalysis.moverRecall` 全部值。
 */
function checkNoPriceLimitWording(files, seedSrc) {
  const violations = [];
  let scanned = 0;
  for (const { name, src } of files) {
    scanned += 1;
    const stripped = stripCommentLines(src);
    if (stripped.includes("涨停")) {
      const line = stripped.split("\n").findIndex((l) => l.includes("涨停")) + 1;
      violations.push(`${name}:${line} 出现「涨停」字样 ⇒ 口径是绝对涨幅，名大于内容即为歧义`);
    }
  }
  const descs = moverVarDescriptions(seedSrc);
  if (descs.length < MOVER_TIERS.length) {
    violations.push(`${SEED_VARS}: 只解析到 ${descs.length}/4 条 mover_gain_* 变量描述 ⇒ 判据失效`);
  }
  for (const [tier, desc] of descs) {
    scanned += 1;
    if (desc.includes("涨停")) {
      violations.push(`${SEED_VARS}: mover_gain_${tier} 的变量描述写「涨停」⇒ 阈值文案与绝对涨幅口径矛盾`);
    }
  }
  for (const lang of LOCALE_LANGS) {
    scanned += 1;
    try {
      const json = JSON.parse(read(`${LOCALES_DIR}/${lang}.json`));
      const section = JSON.stringify((json.stockAnalysis ?? {}).moverRecall ?? {});
      if (section.includes("涨停")) {
        violations.push(`${lang}.json: moverRecall 文案出现「涨停」⇒ 与绝对涨幅口径矛盾`);
      }
    } catch {
      violations.push(`${lang}.json 解析失败 ⇒ 无法核对口径边界`);
    }
  }
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
  /** 门 k/l 的种子夹具：默认四档与引擎出厂一致；`vals` / `descs` 覆盖单档。 */
  const seedFixture = (vals = {}, descs = {}) =>
    MOVER_TIERS.map((t) => {
      const def = { ultra_short: 10.0, short: 20.0, mid: 30.0, long: 40.0 };
      return `        Variable {\n`
        + `            name: "mover_gain_${t}".into(),\n`
        + `            var_type: "number".into(),\n`
        + `            value: serde_json::json!(${vals[t] ?? def[t]}),\n`
        + `            description: Some("${descs[t] ?? `第 ${t} 档窗口累计涨幅达标阈值（%）`}"),\n`
        + `        },`;
    }).join("\n");
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
      got: checkAbsenceReasonTranslated(
        'pub const MATRIX: [Cell; 1] = [("trend", "long", Some("brand_new_absence_code"))];',
      ).violations.length,
      want: 11,
    },
    {
      name: "g 负控（新错配码没翻译）应红",
      got: checkAbsenceReasonTranslated(
        'pub const MATRIX: [Cell; 1] = [("trend", "long", None)];\n'
          + 'pub const MISFIT_DECLARATIONS: [(&str, &str, &str); 1] =\n    [("value", "ultra_short", "brand_new_misfit_code")];',
      ).violations.length,
      want: 11,
    },
    {
      // 判据失效自证：常量改名 / 声明体解析不到 ⇒ 必须红，不得静默当成「没有理由码要翻译」。
      // 夹具刻意用 `RENAMED_MATRIX`（含 MATRIX 子串）——它同时锁住「名字必须带词边界」这件事。
      // want=1 的依据：只报「判据失效」这一条；集合里剩下的 `cell_not_in_matrix` 是真表里
      // **已有翻译**的兜底码，所以不该重复计红（12 是我第一次算错的样子，留此备注防再算错）。
      name: "g 负控（MATRIX 声明体解析不到）应红",
      got: checkAbsenceReasonTranslated('pub const RENAMED_MATRIX: [Cell; 1] = [("a", "b", None)];')
        .violations.length,
      want: 1,
    },
    {
      // 本门的扫描面判据本身：契约码只从声明体取，测试正文里的 `Some("cell_is_active")`
      // 是「出票」哨兵而不是理由码，不能拿它去要翻译（本仓实测被它红过一次）
      name: "g 正控（哨兵码不出现在声明体）应绿",
      got: checkAbsenceReasonTranslated(
        'pub const MATRIX: [Cell; 1] = [("trend", "long", None)];\n#[cfg(test)] mod tests { fn t() { assert_eq!(reason, Some("cell_is_active")); } }',
      ).violations.length,
      want: 0,
    },
    {
      // 修复前的工作流链真实形态：固定乘数出止损/目标 + 落库无来源键（策略链用现盘文件，0 违规）
      name: "h 负控（工作流链仍用固定乘数）应红",
      got: checkSerenityRiskParity(
        '        let (entry_low, entry_high, stop_loss, target_price) = if price > 0.0 {\n'
          + "            (price * (1.0 - serenity_entry_range), price * serenity_stop_mult, price * serenity_target_mult)\n"
          + '        };\n                        "positionPct": 5.0,\n                        "riskNotes": [],\n',
        read(SERENITY_STRATEGY),
        read(RECO_RISK),
      ).violations.length,
      min: 15,
    },
    {
      name: "h 负控（策略链静默放行涨幅过滤）应红",
      got: checkSerenityRiskParity(
        read(SERENITY_WORKFLOW),
        '        if let Ok(klines) = client.get_klines_with_adj(code, "daily", 252, None).await {\n'
          + "            if let Some(first) = klines.first() {\n                let gain = (latest - first.close) / first.close;\n            }\n        }\n",
        read(RECO_RISK),
      ).violations.length,
      min: 3,
    },
    {
      // 建仓带公式被抄回链文件（Q1=B 的「一处实现」塌成两处）⇒ 必须红
      name: "h 负控（两链各自内联建仓带公式）应红",
      got: checkSerenityRiskParity(
        '                        let (entry_low, entry_high) = (price * (1.0 - used_stop_pct / 2.0), price * (1.0 + used_stop_pct / 2.0));\n',
        '        let (entry_low, entry_high) = (price * (1.0 - entry_half_pct / 2.0), price);\n',
        "",
      ).violations.length,
      min: 2,
    },
    {
      name: "h 正控（当前两链形态）应绿",
      got: checkSerenityRiskParity(
        read(SERENITY_WORKFLOW),
        read(SERENITY_STRATEGY),
        read(RECO_RISK),
      ).violations.length,
      want: 0,
    },
    {
      // Q2 落地前的真实文案（面板已随双档改造，这段是它当时的样子）：
      // 单档声明 + 抄了权威表的 28 天 ⇒ 三条各红一次（恒为 mid、缺长线、抄天数）
      name: "i 负控（文案仍声明单档且抄权威天数）应红",
      got: checkTierScopeHint(
        "档位口径：仅服务中线（瓶颈/政策/业绩催化剂以周-月兑现）；短/超短档不适用，落库 period 恒为 mid、持有期按该档权威 28 天。",
      ).violations.length,
      want: 4,
    },
    {
      name: "i 负控（文案为空 ⇒ 无从核对）应红",
      got: checkTierScopeHint("").violations.length,
      want: 1,
    },
    {
      name: "i 正控（当前 zh-CN 文案与双档一致）应绿",
      got: checkTierScopeHint(serenityTierScopeHint()).violations.length,
      want: 0,
    },
    {
      // A 裁定前的卡片真实形态：`?? 20` 兜底 + 无档徽标 + 时间基线不分支
      name: "j 负控（卡片无档徽标且兜底 20 天）应红",
      got: checkSerenityCardTierDisplay(
        '  const holdingDays = candidate.holdingDays ?? candidate.holding_days ?? 20;\n'
          + '  {t("serenityPanel.timeBasis", { date: basisDate, days: holdingDays, until: basisUntil })}',
      ).violations.length,
      want: 5,
    },
    {
      name: "j 正控（当前卡片形态）应绿",
      got: checkSerenityCardTierDisplay(read(SERENITY_CARD)).violations.length,
      want: 0,
    },
    {
      // 修复前真实形态：同一条阈值在引擎常量表与模板变量种子里各写一份
      name: "k 负控（引擎出厂阈值与种子默认漂移）应红",
      got: checkMoverThresholdSingleSource(
        'pub const DEFAULT_GAIN_THRESHOLDS: [(&str, f64); 4] = [\n'
          + '    ("mover_gain_ultra_short", 10.0),\n'
          + '    ("mover_gain_short", 25.0),\n'
          + '    ("mover_gain_mid", 30.0),\n'
          + '    ("mover_gain_long", 40.0),\n'
          + '];\n'
          + 'let window_days = period.default_holding_days();',
        seedFixture(),
      ).violations.length,
      want: 1,
    },
    {
      name: "k 负控（四档缺档登记）应红",
      got: checkMoverThresholdSingleSource(
        'pub const DEFAULT_GAIN_THRESHOLDS: [(&str, f64); 3] = [\n'
          + '    ("mover_gain_ultra_short", 10.0),\n'
          + '    ("mover_gain_short", 20.0),\n'
          + '    ("mover_gain_mid", 30.0),\n'
          + '];\n'
          + 'let window_days = period.default_holding_days();',
        seedFixture(),
      ).violations.length,
      want: 1,
    },
    {
      name: "k 负控（window_days 写死字面量）应红",
      got: checkMoverThresholdSingleSource(
        'pub const DEFAULT_GAIN_THRESHOLDS: [(&str, f64); 4] = [\n'
          + '    ("mover_gain_ultra_short", 10.0),\n'
          + '    ("mover_gain_short", 20.0),\n'
          + '    ("mover_gain_mid", 30.0),\n'
          + '    ("mover_gain_long", 40.0),\n'
          + '];\n'
          + 'let window_days = period.default_holding_days();\n'
          + 'TierRule { period, var_name, gain_pct, window_days: 5 }',
        seedFixture(),
      ).violations.length,
      want: 1,
    },
    {
      name: "k 负控（常量声明解析不到 ⇒ 判据失效）应红",
      got: checkMoverThresholdSingleSource("// 空\n", "// 空\n").violations.length,
      want: 1,
    },
    {
      name: "k 正控（当前真实文件）应绿",
      got: checkMoverThresholdSingleSource(read(MOVER_ENGINE), read(SEED_VARS)).violations.length,
      want: 0,
    },
    {
      name: "l 负控（呈现代码写「涨停」）应红",
      got: checkNoPriceLimitWording(
        [{ name: "x.tsx", src: '  <span>{t("stockAnalysis.moverRecall.rule")}：涨停</span>' }],
        seedFixture(),
      ).violations.length,
      want: 1,
    },
    {
      name: "l 负控（变量描述写「涨停」）应红",
      got: checkNoPriceLimitWording(
        [{ name: "x.rs", src: "let a = 1;\n" }],
        seedFixture({}, { ultra_short: "超短档窗口累计涨幅达标阈值（%，判据不含板块涨停语义）" }),
      ).violations.length,
      want: 1,
    },
    {
      name: "l 正控（注释里写「涨停」不计入 + 真实文件全绿）应绿",
      got: checkNoPriceLimitWording(
        [{ name: "x.rs", src: "// 口径边界：判据不含板块涨停语义\nlet a = 1;\n" }],
        seedFixture(),
      ).violations.length
        + checkNoPriceLimitWording(
          [MOVER_ENGINE, MOVER_CMD, MOVER_PANEL].map((f) => ({ name: f, src: read(f) })),
          read(SEED_VARS),
        ).violations.length,
      want: 0,
    },
  ];
  let bad = 0;
  for (const c of cases) {
    // `min`：只要求「至少红 N 条」——用于**多判据复合夹具**（逐条数违规数我连着算错两次，
    // 每次算错都要回头改夹具而不是改生产，那种门会把人训练成调数字）。精确数仍用 `want`。
    const ok = c.min === undefined ? c.got === c.want : c.got >= c.min;
    if (!ok) { bad += 1; }
    const budget = c.min === undefined ? `want=${c.want}` : `min=${c.min}`;
    console.log(`${ok ? "PASS" : "FAIL"} ${c.name}（got=${c.got} ${budget}）`);
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
        ? checkSerenityRiskParity(
            read(SERENITY_WORKFLOW),
            read(SERENITY_STRATEGY),
            read(RECO_RISK),
          )
        : { violations: [`${SERENITY_WORKFLOW} 或 策略链文件不存在 ⇒ 无从比对`], scanned: 0 },
    ],
    ["i 面板档位口径文案与代码一致", checkTierScopeHint(serenityTierScopeHint())],
    [
      "j 趋势智选卡片呈现档位与风控",
      fs.existsSync(path.join(ROOT, SERENITY_CARD))
        ? checkSerenityCardTierDisplay(read(SERENITY_CARD))
        : { violations: [`${SERENITY_CARD} 不存在`], scanned: 0 },
    ],
    [
      "k 窗口涨幅达标阈值/天数单源",
      [MOVER_ENGINE, SEED_VARS].every((f) => fs.existsSync(path.join(ROOT, f)))
        ? checkMoverThresholdSingleSource(read(MOVER_ENGINE), read(SEED_VARS))
        : { violations: [`${MOVER_ENGINE} 或 ${SEED_VARS} 不存在 ⇒ 无从比对阈值单源`], scanned: 0 },
    ],
    [
      "l 口径边界（用户可见文案不得出现「涨停」）",
      [MOVER_ENGINE, MOVER_CMD, MOVER_PANEL, SEED_VARS].every((f) => fs.existsSync(path.join(ROOT, f)))
        ? checkNoPriceLimitWording(
            [MOVER_ENGINE, MOVER_CMD, MOVER_PANEL].map((f) => ({ name: f, src: read(f) })),
            read(SEED_VARS),
          )
        : { violations: ["mover 链文件缺失 ⇒ 口径边界无从核对"], scanned: 0 },
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
