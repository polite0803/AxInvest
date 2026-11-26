#!/usr/bin/env node
/**
 * 四周期**档名**一致性门禁（`AUDIT-horizon-naming-2026-10-16.md`）。
 *
 * ## 为什么需要它
 *
 * 2026-10-16 实测：档名（中文标签）这条链**整体没有单一真相门** ——
 * 权威 `Period::label_zh()` 与前端 **8 套 i18n 键族**之间**零对账**，于是同一档
 * 可以在不同屏上叫不同名字而全仓「编译通过 + 全门绿」：
 *
 * | 事实 | 形态 |
 * |---|---|
 * | 曾经的 8 套键族 | `stockAnalysis.period.*` / `period*` / `timeHorizon*` / `recommendation.period*` / `reflection.horizon*` / `scheduledAnalysis.period.*`、`replayWorkbench.batch.period.*`、`settings.scheduled-recommendation.period-*` |
 * | 修复前的漂移 | 第 1 档 `超短` vs `超短线`、第 2 档 `短期` vs `短线`、第 3 档 `中期` vs `中线`；非中文语言里 ar/hi/ja 的多套键族**整族是未翻译的英文占位**（`Short \| Short \| Mid`） |
 * | 第 4 档事故 | 按「90 天 = 一个季度」把对外名改成「季度」，而 `quarterly` 是**尺度名**（`Scale::Quarterly` = 60 交易日/bar）、持有期是 **90 交易日** ⇒ 相差 1.5 倍；产出「季度 90 天」自相矛盾文案 + 前后端同屏两名（后端 `tierLabel`/`dataGaps` 显示「长线」） |
 *
 * 既有门为何没拦住：`check-horizon-weight-parity.mjs` 的判据 ④ 只读
 * `stockAnalysis.reflection` 段（8 套键族里**只覆盖 1 套**）；`check-reco-horizon-parity.mjs`
 * 的门 i 当时被**放宽**成认「长线|季度」两种写法（改判据去适配改名）。本门补上整条链。
 *
 * ## 收敛（同日第二批）
 *
 * 值统一之后，**定义仍是 8 份** ⇒ 任一族漏改仍会前后端同屏两名。当日把 7 套冗余族
 * **整体删除**（28 键 × 11 语言 + 2 个被删空的壳对象），消费端一律改走单一存取器
 * `horizonLabelKey()`（`src/lib/stock-analysis-utils.ts`）。此后本门的主判据从
 * 「族间一致（A）」换成「**唯一族 + 禁回流（E）**」——防的不是不一致，是**重复定义再生**。
 *
 * ## 判据
 *
 * - **判据 0**：任何判据扫描面为 0 ⇒ 判红（「扫到 0 条」的门等于没有门）。
 * - **判据 B（禁尺度词）**：四档值不得含**尺度名**系词（季度 / Quarterly / 四半期 /
 *   분기 / Quartal / Trimestre / Квартальный / ربع سنوي / त्रैमासिक）。
 *   尺度是 bar 粒度、持有期是交易日数，两者不是一个量 —— 用尺度名当档名会把
 *   「持有 90 交易日」读成「持有 1 个季度」。
 * - **判据 C（权威对账）**：zh-CN 四档必须**逐字等于** `Period::label_zh()` 四臂右值；
 *   zh-TW 必须等于其显式繁简映射（映射表登记在本文件，不猜）。`label_zh` 解析不出 ⇒ 判红。
 * - **判据 D（e2e 锚点对账）已退役（2026-11-26）**：原扫
 *   `e2e/stock-reflection-four-horizon.spec.ts` 的 `horizonTab(..., "🟢 …")` 硬编码档名锚点。
 *   该 spec 的断言对象（宿主反思面板）随 T3 股票页外迁删除（见
 *   `PLAN-shell-plus-plugins.md`「CI run 45721938652 三红修复」），消费方没了 ⇒
 *   判据 D 连同 0/D 失效检查、`stripCommentLines` / `parseE2eTierAnchors` 一并下线。
 *   档名对账面回归为 i18n 8 套 + 权威 `label_zh`；反思面板的 UI 对账待 stock 仓
 *   web 测试基建重建后随迁移恢复。
 * - **判据 E（唯一族 + 禁回流）**：11 语言里，`RETIRED_FAMILIES` 的 **28 条路径必须全部不存在**。
 *   任何一条复活 ⇒ 判红（「值一样」不是理由：两份定义就是两条会各自腐烂的链）。
 * - **判据 F（四档不得同值）**：单一族的四档值必须**互不相同** —— 4 档抄成同一串
 *   （常见于「先占位后忘改」）会让用户看到四个一模一样的档名，而判据 C 只看 zh 逐字相等，
 *   在「权威自己也是四档同值」时不报；本判据是那格空白的兜底。
 *
 * ## 为什么不设「消费端禁回流」判据（曾写过一版，撤回）
 *
 * 直觉上该加一条「`src/**` 里不得再出现已删族的字面量」——毕竟 **i18n 键是无类型字符串**，
 * `tsc` 不会为 `t("stockAnalysis.period.short")` 报错。但**已有门覆盖了它**：
 * `scripts/check_i18n.py --strict` 的 `missing = code_keys - zh_keys`（第 187 行）就是
 * 「代码引用但 zh-CN 缺失」，且已注册在 `ci-check.mjs:316` 与 `.github/workflows/ci.yml:383`。
 * 实测（2026-10-16，探针文件已删）：往 `src/` 放一句 `t("stockAnalysis.period.short")` ⇒
 * 该门输出「❌ 代码引用但 zh-CN 缺失 (1): stockAnalysis.period.short」并 exit 1。
 *
 * 故本条**不重复实现**（本仓纪律：同型缺陷别铺 N 份实现）。撤回的版本还有一个反面教材：
 * 它自己写状态机剥注释，被 `stock-analysis-utils.ts:231` 一个**含引号的正则**
 * `/["']?\s*(看多|看空|…)/i` 带偏 —— 字符类里的 `'` 被当成字符串起点，其后 5.6 万字符
 * 全被当字符串 ⇒ 注释没剥掉 ⇒ 对一句**文档注释**误报。判据的测量工具自己错，比缺陷更贵。
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
const HOLDING = path.join(ROOT, "axagent-core/crates/harness/src/holding_period.rs");

/** **唯一**档名键族的四条完整路径（超短 / 短 / 中 / 长）。
 *  按**完整路径**读，避免同键名误伤（`quant.trades.long` = 「做多」、
 *  `stockAnalysis.period.5m` = K 线粒度，两段都叫 `period` 但不同义）。 */
export const FAMILY = [
  "stockAnalysis.timeHorizonUltraShort",
  "stockAnalysis.timeHorizonShort",
  "stockAnalysis.timeHorizonMid",
  "stockAnalysis.timeHorizonLong",
];

/** 收敛时**已删除**的 7 套冗余族（28 条完整路径）——判据 E 断言它们不得复活。
 *  含两个被删空的壳对象（`scheduledAnalysis.period` / `replayWorkbench.batch.period`）。 */
export const RETIRED_FAMILIES = [
  ["stockAnalysis.period.ultraShort", "stockAnalysis.period.short", "stockAnalysis.period.mid", "stockAnalysis.period.long"],
  ["stockAnalysis.periodUltraShort", "stockAnalysis.periodShort", "stockAnalysis.periodMid", "stockAnalysis.periodLong"],
  ["stockAnalysis.recommendation.periodUltraShort", "stockAnalysis.recommendation.periodShort", "stockAnalysis.recommendation.periodMid", "stockAnalysis.recommendation.periodLong"],
  ["stockAnalysis.reflection.horizonUltraShort", "stockAnalysis.reflection.horizonShort", "stockAnalysis.reflection.horizonMid", "stockAnalysis.reflection.horizonLong"],
  ["stockAnalysis.scheduledAnalysis.period.ultraShort", "stockAnalysis.scheduledAnalysis.period.short", "stockAnalysis.scheduledAnalysis.period.mid", "stockAnalysis.scheduledAnalysis.period.long"],
  ["replayWorkbench.batch.period.ultra_short", "replayWorkbench.batch.period.short", "replayWorkbench.batch.period.mid", "replayWorkbench.batch.period.long"],
  ["settings.scheduled-recommendation.period-ultra-short", "settings.scheduled-recommendation.period-short", "settings.scheduled-recommendation.period-mid", "settings.scheduled-recommendation.period-long"],
];

/** 已删空的壳对象路径：判据 E 也盯它们（单删 4 键而留 `"period": {}` 是残渣）。 */
export const RETIRED_SHELLS = ["stockAnalysis.scheduledAnalysis.period", "replayWorkbench.batch.period"];

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

/** 读全部 locale：`Map<lang, { values: (string|undefined)[4], retired: string[] }>`。
 *  `values` = 唯一族四档；`retired` = 该语言里**仍存在**的已退休族路径（判据 E 的判据面）。 */
export function readFamilies(localeDir = LOCALE_DIR) {
  const out = new Map();
  for (const f of fs.readdirSync(localeDir).filter((x) => x.endsWith(".json")).sort()) {
    const lang = f.slice(0, -5);
    const j = JSON.parse(fs.readFileSync(path.join(localeDir, f), "utf8"));
    const retired = [...RETIRED_FAMILIES.flat(), ...RETIRED_SHELLS].filter((p) => get(j, p) !== undefined);
    out.set(lang, { values: FAMILY.map((p) => get(j, p)), retired });
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

  // 判据 0：禁回流面本身不得被抽空（把清单删空 = 让判据 E 恒绿）
  const retiredAll = [...RETIRED_FAMILIES.flat(), ...RETIRED_SHELLS];
  if (retiredAll.length !== 28 + RETIRED_SHELLS.length) {
    violations.push(
      `[判据 0/E] 已退休清单只有 ${retiredAll.length} 条（应 ${28 + RETIRED_SHELLS.length}）`
        + ` ⇒ 禁回流面被抽空，判据 E 恒绿`,
    );
  }

  for (const [lang, rec] of families) {
    const values = rec.values;

    // 判据 0：路径面
    const missingPaths = values.map((v, ti) => (v === undefined ? FAMILY[ti] : null)).filter(Boolean);
    if (missingPaths.length) {
      violations.push(`[判据 0] ${lang}：${missingPaths.length} 条档名路径读不到 ⇒ 覆盖面失效（${missingPaths.slice(0, 3).join(", ")}…）`);
      continue;
    }

    // 判据 F：四档值必须互不相同（4 档抄成同一串 = 「先占位后忘改」的典型）
    scanned += 1;
    if (new Set(values).size !== 4) {
      violations.push(
        `[判据 F] ${lang} 四档档名出现同值：${values.map((v) => `"${v}"`).join(" / ")}`
          + ` ⇒ 用户会在四档看到同一串名字`,
      );
    }

    // 判据 B：禁尺度词
    for (let ti = 0; ti < 4; ti++) {
      scanned += 1;
      if (SCALE_WORDS.test(values[ti])) {
        violations.push(
          `[判据 B] ${lang} 第 ${ti + 1} 档档名 "${values[ti]}" 含**尺度名**系词 ⇒ 把 bar 粒度当成了持有期`
            + `（Quarterly = 60 交易日/bar，而第 4 档持有 90 交易日，差 1.5 倍）`,
        );
      }
    }

    // 判据 C：zh-CN / zh-TW 与权威逐字对账
    if (lang === "zh-CN" || lang === "zh-TW") {
      const expect = lang === "zh-CN"
        ? authority.values
        : authority.values.map((v) => ZH_TW_MAP[v] ?? `⟨无繁简映射:${v}⟩`);
      for (let ti = 0; ti < 4; ti++) {
        scanned += 1;
        if (values[ti] !== expect[ti]) {
          violations.push(
            `[判据 C] ${lang} 第 ${ti + 1} 档档名 "${values[ti]}" ≠ 权威 \`Period::label_zh()\` 的 "${expect[ti]}"`
              + ` ⇒ 前后端同屏两名（后端 \`tierLabel\`/\`dataGaps\` 用权威串）`,
          );
        }
        if (expect[ti].includes("⟨无繁简映射")) {
          violations.push(`[判据 0/C] ${lang}：权威值 "${authority.values[ti]}" 未登记 zh-TW 繁简映射 ⇒ 判据 C 失效`);
        }
      }
    }

    // 判据 E：唯一族 —— 已退休的 7 套族与 2 个壳不得复活
    for (const p of rec.retired) {
      scanned += 1;
      violations.push(
        `[判据 E] ${lang}：已退休的档名族路径 "${p}" 又出现了 ⇒ 档名重新变成多份定义`
          + `（「值恰好一样」不是理由：两份定义就是两条会各自腐烂的链，应改走 \`horizonLabelKey()\`）`,
      );
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
    `比对面：${families.size} 语言 × **唯一族** 4 档`
      + ` | 已退休族 ${RETIRED_FAMILIES.flat().length} 键 + ${RETIRED_SHELLS.length} 壳（须全部不存在）`
      + ` | 权威 \`label_zh\` = `
      + (authority.ok ? authority.values.map((v) => `"${v}"`).join(" / ") : "❌ 解析失败")
      + ` | 扫描点 ${scanned}`,
  );
  if (violations.length === 0) {
    console.log("✅ 通过：档名只有一族、7 套冗余族未回流、档名不含尺度词、四档互异、zh 与 `Period::label_zh()` 逐字对账");
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

  /** 深拷贝夹具（每条正控只动一处，互不污染）。 */
  const clone = (m) => new Map([...m].map(([k, v]) => [k, { values: [...v.values], retired: [...v.retired] }]));

  cases.push({
    name: "负控：磁盘当前形态应无违例",
    families: disk,
    auth: authority,
    wantRed: false,
  });

  // 正控 B：把第 4 档改回「季度」（2026-10-16 事故原形）
  const famB = clone(disk);
  famB.get("zh-CN").values[3] = "季度";
  cases.push({
    name: "正控 B：zh-CN 第 4 档改成「季度」⇒ 须报 B（尺度词）",
    families: famB,
    auth: authority,
    wantRed: true,
    wantIn: ["判据 B", "季度"],
  });

  // 正控 E：**本门新增的主判据** —— 冗余族回流（收敛后最该防的形态）
  const famE = clone(disk);
  famE.get("zh-TW").retired.push("stockAnalysis.period.short");
  cases.push({
    name: "正控 E：已退休族路径复活（哪怕值一样）⇒ 须报禁回流",
    families: famE,
    auth: authority,
    wantRed: true,
    wantIn: ["判据 E", "stockAnalysis.period.short"],
  });

  // 正控 F：四档同值（「先占位后忘改」）
  const famF = clone(disk);
  famF.get("de").values = ["Kurzfristig", "Kurzfristig", "Kurzfristig", "Kurzfristig"];
  cases.push({
    name: "正控 F：某语言四档抄成同一串 ⇒ 须报四档同值",
    families: famF,
    auth: authority,
    wantRed: true,
    wantIn: ["判据 F"],
  });

  // 正控 C：zh-CN 与权威脱钩（最隐蔽：判据 F 也过，只有 C 能抓）
  const famC = clone(disk);
  famC.get("zh-CN").values[0] = "超短";
  cases.push({
    name: "正控 C：zh-CN 把第 1 档写成「超短」⇒ 判据 C 必须抓到",
    families: famC,
    auth: authority,
    wantRed: true,
    wantIn: ["判据 C"],
  });

  // 正控 0：权威解析失败不得当「没有这条规则」
  cases.push({
    name: "正控 0：label_zh 解析失败 ⇒ 判红",
    families: disk,
    auth: { ok: false, reason: "夹具：函数被改名" },
    wantRed: true,
    wantIn: ["判据 0"],
  });

  // 正控 0：语言面不足
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
  const nPos = cases.filter((c) => c.wantRed).length;
  if (bad === 0) {
    console.log(`\n✅ 自检通过：${nPos} 个正控全部报红、${cases.length - nPos} 个负控为绿 ⇒ 判据具区分力`);
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
