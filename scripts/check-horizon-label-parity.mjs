#!/usr/bin/env node
/**
 * 四周期**档名**一致性门禁（`AUDIT-horizon-naming-2026-10-16.md`）。
 *
 * ## 为什么需要它
 *
 * 2026-10-16 实测：档名（中文标签）这条链**整体没有单一真相门** ——
 * 权威 `Period::label_zh()` 与前端 8 套 i18n 键族之间**零对账**，于是同一档
 * 可以在不同屏上叫不同名字而全仓「编译通过 + 全门绿」：
 *
 * | 事实 | 形态 |
 * |---|---|
 * | 前端 8 套键族 | `stockAnalysis.period.*` / `period*` / `timeHorizon*` / `recommendation.period*` / `reflection.horizon*` / `scheduledAnalysis.period.*`、`replayWorkbench.batch.period.*`、`settings.scheduled-recommendation.period-*` |
 * | 修复前的漂移 | 第 1 档 `超短` vs `超短线`、第 2 档 `短期` vs `短线`、第 3 档 `中期` vs `中线`；非中文语言里 ar/hi/ja 的多套键族**整族是未翻译的英文占位**（`Short \| Short \| Mid`） |
 * | 第 4 档事故 | 按「90 天 = 一个季度」把对外名改成「季度」，而 `quarterly` 是**尺度名**（`Scale::Quarterly` = 60 交易日/bar）、持有期是 **90 交易日** ⇒ 相差 1.5 倍；产出「季度 90 天」自相矛盾文案 + 前后端同屏两名（后端 `tierLabel`/`dataGaps` 显示「长线」） |
 *
 * 既有门为何没拦住：`check-horizon-weight-parity.mjs` 的判据 ④ 只读
 * `stockAnalysis.reflection` 段（8 套键族里**只覆盖 1 套**）；`check-reco-horizon-parity.mjs`
 * 的门 i 当时被**放宽**成认「长线|季度」两种写法（改判据去适配改名）。本门补上整条链。
 *
 * ## 判据
 *
 * - **判据 0**：任何判据扫描面为 0 ⇒ exit 2（「扫到 0 条」的门等于没有门）。
 * - **判据 A（族间一致）**：同一语言内，8 套键族的**四档值必须逐字相同**。
 *   跨族漂移正是「重复定义」的载体：值不同 ⇒ 用户在不同面板看到不同档名。
 * - **判据 B（禁尺度词）**：四档值不得含**尺度名**系词（季度 / Quarterly / 四半期 /
 *   분기 / Quartal / Trimestre / Квартальный / ربع سنوي / त्रैमासिक）。
 *   尺度是 bar 粒度、持有期是交易日数，两者不是一个量 —— 用尺度名当档名会把
 *   「持有 90 交易日」读成「持有 1 个季度」。
 * - **判据 C（权威对账）**：zh-CN 四档必须**逐字等于** `Period::label_zh()` 四臂右值；
 *   zh-TW 必须等于其显式繁简映射（映射表登记在本文件，不猜）。`label_zh` 解析不出 ⇒ 判红。
 *
 * 用法：
 *   node scripts/check-horizon-label-parity.mjs            # 比对（违例 exit 1）
 *   node scripts/check-horizon-label-parity.mjs --selftest # 正负对照（复刻事故形态，证明会红）
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const LOCALE_DIR = path.join(ROOT, "src/i18n/locales");
const HOLDING = path.join(ROOT, "src-tauri/crates/harness/src/holding_period.rs");

/** 8 套档名键族的完整路径（超短 / 短 / 中 / 长）。按**完整路径**读，避免同键名误伤
 *  （`quant.trades.long` = 「做多」、`quant.trades.short` = 「做空」与档名同键名）。 */
export const FAMILIES = [
  ["stockAnalysis.period.ultraShort", "stockAnalysis.period.short", "stockAnalysis.period.mid", "stockAnalysis.period.long"],
  ["stockAnalysis.periodUltraShort", "stockAnalysis.periodShort", "stockAnalysis.periodMid", "stockAnalysis.periodLong"],
  ["stockAnalysis.timeHorizonUltraShort", "stockAnalysis.timeHorizonShort", "stockAnalysis.timeHorizonMid", "stockAnalysis.timeHorizonLong"],
  ["stockAnalysis.recommendation.periodUltraShort", "stockAnalysis.recommendation.periodShort", "stockAnalysis.recommendation.periodMid", "stockAnalysis.recommendation.periodLong"],
  ["stockAnalysis.reflection.horizonUltraShort", "stockAnalysis.reflection.horizonShort", "stockAnalysis.reflection.horizonMid", "stockAnalysis.reflection.horizonLong"],
  ["stockAnalysis.scheduledAnalysis.period.ultraShort", "stockAnalysis.scheduledAnalysis.period.short", "stockAnalysis.scheduledAnalysis.period.mid", "stockAnalysis.scheduledAnalysis.period.long"],
  ["replayWorkbench.batch.period.ultra_short", "replayWorkbench.batch.period.short", "replayWorkbench.batch.period.mid", "replayWorkbench.batch.period.long"],
  ["settings.scheduled-recommendation.period-ultra-short", "settings.scheduled-recommendation.period-short", "settings.scheduled-recommendation.period-mid", "settings.scheduled-recommendation.period-long"],
];

/** 档位顺序（与 `Period::ALL` 变体声明序一致：短 → 长）。 */
export const TIER_NAMES = ["超短", "短", "中", "长"];

/** 尺度名系词 —— 出现在**档名**里即为「尺度/持有期混淆」。 */
export const SCALE_WORDS = /季度|四半期|Quarterly|Quartal|Trimestre|Trimestral|ربع سنوي|त्रैमासिक|분기|Квартальн/i;

/** zh-TW 显式繁简映射（登记而非猜；`label_zh` 是简体权威）。 */
export const ZH_TW_MAP = { 超短线: "超短線", 短线: "短線", 中线: "中線", 长线: "長線" };

const get = (o, k) => k.split(".").reduce((a, p) => (a == null ? undefined : a[p]), o);

/** 读 `Period::label_zh()` 的四个臂右值。取不到 ⇒ `ok:false`（调用侧必须判红）。 */
export function parseLabelZh(src) {
  const anchor = "pub fn label_zh(&self) -> &'static str {";
  const i = src.indexOf(anchor);
  if (i < 0) { return { ok: false, reason: `找不到 \`${anchor}\`` }; }
  const seg = src.slice(i, i + 600);
  const found = new Map();
  for (const m of seg.matchAll(/Period::(UltraShort|Short|Mid|Long)\s*=>\s*"([^"]+)"/g)) {
    found.set(m[1], m[2]);
  }
  const order = ["UltraShort", "Short", "Mid", "Long"];
  const missing = order.filter((o) => !found.has(o));
  if (missing.length) { return { ok: false, reason: `label_zh 缺臂：${missing.join(" / ")}` }; }
  return { ok: true, values: order.map((o) => found.get(o)) };
}

/** 读全部 locale 的 8 套键族四档值。返回 `Map<lang, string[][]>`（族 × 档）。 */
export function readFamilies(localeDir = LOCALE_DIR) {
  const out = new Map();
  for (const f of fs.readdirSync(localeDir).filter((x) => x.endsWith(".json")).sort()) {
    const lang = f.slice(0, -5);
    const j = JSON.parse(fs.readFileSync(path.join(localeDir, f), "utf8"));
    out.set(lang, FAMILIES.map((fam) => fam.map((p) => get(j, p))));
  }
  return out;
}

/** 纯判据（与 IO 解耦，供 `--selftest` 用内存夹具驱动）。 */
export function judge(families, authority) {
  const violations = [];
  let scanned = 0;

  // 判据 0：权威解析面
  if (!authority.ok) {
    violations.push(`[判据 0/C] 档名权威解析失败：${authority.reason} ⇒ 判红而不是当没有`);
    return { violations, scanned };
  }

  // 判据 0：语言面
  if (families.size < 11) {
    violations.push(`[判据 0] 只读到 ${families.size} 个语言文件（< 11）⇒ 覆盖面失效`);
    return { violations, scanned };
  }

  for (const [lang, rows] of families) {
    // 判据 0：路径面
    const missingPaths = rows.flatMap((r, i) => r.map((v, ti) => (v === undefined ? FAMILIES[i][ti] : null)).filter(Boolean));
    if (missingPaths.length) {
      violations.push(`[判据 0] ${lang}：${missingPaths.length} 条档名路径读不到 ⇒ 覆盖面失效（${missingPaths.slice(0, 3).join(", ")}…）`);
      continue;
    }

    // 判据 A：同语言内跨族逐字一致
    for (let ti = 0; ti < 4; ti++) {
      scanned += 1;
      const vals = [...new Set(rows.map((r) => r[ti]))];
      if (vals.length > 1) {
        violations.push(
          `[判据 A] ${lang} 第 ${ti + 1} 档（${TIER_NAMES[ti]}）跨键族不一致：${vals.map((v) => `"${v}"`).join(" / ")}`
            + ` ⇒ 同一档在不同面板显示不同名字（8 套键族须逐字同一串）`,
        );
      }
    }

    // 判据 B：禁尺度词
    for (let ti = 0; ti < 4; ti++) {
      for (const v of new Set(rows.map((r) => r[ti]))) {
        scanned += 1;
        if (SCALE_WORDS.test(v)) {
          violations.push(
            `[判据 B] ${lang} 第 ${ti + 1} 档档名 "${v}" 含**尺度名**系词 ⇒ 把 bar 粒度当成了持有期`
              + `（Quarterly = 60 交易日/bar，而第 4 档持有 90 交易日，差 1.5 倍）`,
          );
        }
      }
    }

    // 判据 C：zh-CN / zh-TW 与权威逐字对账
    if (lang === "zh-CN" || lang === "zh-TW") {
      const expect = lang === "zh-CN"
        ? authority.values
        : authority.values.map((v) => ZH_TW_MAP[v] ?? `⟨无繁简映射:${v}⟩`);
      const row = rows[0];
      for (let ti = 0; ti < 4; ti++) {
        scanned += 1;
        if (row[ti] !== expect[ti]) {
          violations.push(
            `[判据 C] ${lang} 第 ${ti + 1} 档档名 "${row[ti]}" ≠ 权威 \`Period::label_zh()\` 的 "${expect[ti]}"`
              + ` ⇒ 前后端同屏两名（后端 \`tierLabel\`/\`dataGaps\` 用权威串）`,
          );
        }
        if (expect[ti].includes("⟨无繁简映射")) {
          violations.push(`[判据 0/C] ${lang}：权威值 "${authority.values[ti]}" 未登记 zh-TW 繁简映射 ⇒ 判据 C 失效`);
        }
      }
    }
  }

  // 判据 0：扫描面非零
  if (scanned === 0) {
    violations.push("[判据 0] 扫描面为 0 ⇒ 本门不可信（0 命中 ≠ 没问题）");
  }

  return { violations, scanned };
}

function run() {
  const authority = parseLabelZh(fs.readFileSync(HOLDING, "utf8"));
  const families = readFamilies();
  const { violations, scanned } = judge(families, authority);
  console.log(
    `比对面：${families.size} 语言 × 8 键族 × 4 档 | 权威 \`label_zh\` = `
      + (authority.ok ? authority.values.map((v) => `"${v}"`).join(" / ") : "❌ 解析失败")
      + ` | 扫描点 ${scanned}`,
  );
  if (violations.length === 0) {
    console.log("✅ 通过：8 套键族逐字一致、档名不含尺度词、zh 与 `Period::label_zh()` 逐字对账");
    return 0;
  }
  violations.forEach((v) => console.log(`✖ ${v}`));
  console.log("\n❌ 档名一致性存在违例（权威 = `harness::holding_period::Period::label_zh`）");
  return 1;
}

/** 正负对照：每个正控都在内存里复刻**真实事故形态**，证明判据会红；负控用磁盘当前形态（应绿）。 */
function selftest() {
  const authority = parseLabelZh(fs.readFileSync(HOLDING, "utf8"));
  const disk = readFamilies();
  const cases = [];

  cases.push({
    name: "负控：磁盘当前形态应无违例",
    families: disk,
    auth: authority,
    wantRed: false,
  });

  // 正控 1：判据 B —— 把第 4 档改回「季度」（2026-10-16 事故原形）
  const famB = new Map(disk);
  {
    const rows = famB.get("zh-CN").map((r) => [...r]);
    for (const r of rows) r[3] = "季度";
    famB.set("zh-CN", rows);
  }
  cases.push({
    name: "正控 A/B：zh-CN 第 4 档改成「季度」⇒ 须报 A（族间不一致）与 B（尺度词）",
    families: famB,
    auth: authority,
    wantRed: true,
    wantIn: ["判据 B", "季度"],
  });

  // 正控 2：判据 A —— 只改一族（第 3 档 中期 vs 中线）
  const famA = new Map(disk);
  {
    const rows = famA.get("en-US").map((r) => [...r]);
    rows[4][2] = "Mid Term"; // 只有 reflection 族用旧写法
    famA.set("en-US", rows);
  }
  cases.push({
    name: "正控 A：单族漂移（en-US 第 3 档只改一族）⇒ 须报族间不一致",
    families: famA,
    auth: authority,
    wantRed: true,
    wantIn: ["判据 A"],
  });

  // 正控 3：判据 C —— zh-CN 与权威脱钩但族间自洽（最隐蔽：全族一致地错）
  const famC = new Map(disk);
  {
    const rows = famC.get("zh-CN").map((r) => [...r]);
    for (const r of rows) r[0] = "超短";
    famC.set("zh-CN", rows);
  }
  cases.push({
    name: "正控 C：zh-CN 全族一致地把第 1 档写成「超短」⇒ 判据 A 放过、判据 C 必须抓到",
    families: famC,
    auth: authority,
    wantRed: true,
    wantIn: ["判据 C"],
  });

  // 正控 4：判据 0 —— 权威解析失败不得当「没有这条规则」
  cases.push({
    name: "正控 0：label_zh 解析失败 ⇒ 判红",
    families: disk,
    auth: { ok: false, reason: "夹具：函数被改名" },
    wantRed: true,
    wantIn: ["判据 0"],
  });

  // 正控 5：判据 0 —— 语言面不足
  const famD = new Map([...disk].slice(0, 3));
  cases.push({
    name: "正控 0：只读到 3 个语言 ⇒ 判红",
    families: famD,
    auth: authority,
    wantRed: true,
    wantIn: ["判据 0"],
  });

  let bad = 0;
  for (const c of cases) {
    const { violations } = judge(c.families, c.auth);
    const red = violations.length > 0;
    const hitAll = !c.wantIn || c.wantIn.every((needle) => violations.some((v) => v.includes(needle)));
    const ok = red === c.wantRed && (c.wantRed ? hitAll : true);
    console.log(`${ok ? "✅" : "❌"} ${c.name}（违例 ${violations.length} 条）`);
    if (!ok) {
      bad += 1;
      violations.forEach((v) => console.log(`     · ${v}`));
    }
  }
  if (bad === 0) {
    console.log("\n✅ 自检通过：5 个正控全部报红、负控为绿 ⇒ 判据具区分力");
    return 0;
  }
  console.log(`\n❌ 自检失败 ${bad} 例 ⇒ 门的区分力不可信`);
  return 1;
}

if (process.argv.includes("--selftest")) {
  process.exit(selftest());
} else {
  process.exit(run());
}
