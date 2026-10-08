// 全域反向门：每个**声明**的种子变量都必须有「非声明面引用面」，否则必须显式登记为豁免。
//
// ── 为什么建（2026-10-08 实测）──
// `seed_variables.rs` 声明 374 个变量。按本文件的引用面判据普查，**36 个零引用面**，其中
// **34 个仍被设置面板渲染成可编辑控件** ⇒ 用户改了没有任何东西读它。这正是本仓反复登记的
// 「配置项空接线」族（`is_async` / `cache_*` / 面板技术指标 8 项都属同族）。
// §一○七 待拍板 1 当时预测「全域版会把 Rust 播种期读走的变量误报成无源」——
// **该预测被实测否证**：非声明面引用面判据下只有 36 条候选，逐条可判真伪。
//
// ── 判据（这是**必要条件**的判据，别读成充分条件）──
// G1 每个声明变量：引用面命中 ≥1，或在 `EXEMPT` 登记。新条目 ⇒ 红。
// G2 登记项若**已**有引用面 ⇒ 红（接线了就摘掉豁免，否则门失去真实面积读数）。
// G3 登记项若已不在声明面 ⇒ 红（变量删了、豁免还留着＝假账）。
// G4 每条登记必须有 status ∈ {intentional, pending}；intentional 必须给 cite（书面裁定出处）。
// G5 解析面自检：声明数 / 扫描文件数低于下限 ⇒ 红（「判据没电」不等于「全绿」）。
//
// ⚠ 「有引用面」只证明这个名字在非声明面被**写出来过**，不证明它真被读进执行路径。
//   所以本门是**面积上限**判据：拦「新增空接线」，不保证「已登记的都活着」——
//   后者靠 `check-indicator-config-scope.mjs` 那种定向门（两侧字面量逐字对账）。
//
// ── 用法 ──
//   node scripts/check-variable-consumer-registry.mjs             # 门禁（0/1）
//   node scripts/check-variable-consumer-registry.mjs --selftest   # 七条负控，每条必须真能红/真能判
//   node scripts/check-variable-consumer-registry.mjs --dump       # 逐条点名零引用面变量 + 面板暴露
//
// 退出码：0 通过 ｜ 1 违规 ｜ 2 判据没电（取不到声明面 / 扫描面塌陷）

import { readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const read = (rel) => readFileSync(resolve(ROOT, rel), "utf8");

const SEED_REL = "src-tauri/src/commands/stock_analysis_setup/seed_variables.rs";
const PANEL_REL = "src/components/settings/StockAnalysisConfigPanel.tsx";
const MOD_REL = "src-tauri/src/commands/stock_analysis_setup/mod.rs";

/** 声明面下限：现网 374 —— 解析器坏 / 声明形态改名 ⇒ 立刻判红，不当「无违规」 */
const MIN_DECLARED = 200;
/** 扫描面下限：现网 3500+ 个文件 */
const MIN_SCANNED_FILES = 1500;

/**
 * 零引用面登记（2026-10-08 首跑读数 36 条）。
 * status：`intentional` = 有书面裁定的「刻意不接」（必须给 cite）；`pending` = 待裁定接还是删。
 *
 * ⚠ 2026-10-08 A 批接线**摘掉 9 条**（`signal_rsi_oversold` / `signal_rsi_overbought`、
 *   `pos_max_single_pct` / `pos_max_total` / `pos_max_sector_pct`、`val_pe_low` / `val_pe_high`、
 *   `monitor_poll_interval_secs`、`news_limit`）：它们现在各有真实落点。落点位置与
 *   「面板默认 == Rust 回落默认」的逐字对账锁在定向门 `check-panel-var-landing.mjs` ——
 *   本门只保证「这个名字在非声明面被写出来过」（面积判据），逐字对账不归它管。
 *   剩下的 4 条是 B 批：接上会改决策数值（PB 两档、护城河、安全边际），需单独拍板换代。
 */
export const EXEMPT = [
  // 估值 / 价值策略（B 批）：面板默认与 Rust 现常量**不等** ⇒ 接线即改现网数值。
  // `val_pb_low` 1.5→1.0、`val_pb_high` 5.0→6.0、`value_moat_threshold` 70/40 双档→60、
  // `value_safety_margin` 30%→20%（现值出处见 `PLAN-four-horizon-workflow-alignment.md` §一一六(3)）。
  { name: "val_pb_low", status: "pending" },
  { name: "val_pb_high", status: "pending" },
  { name: "value_moat_threshold", status: "pending" },
  { name: "value_safety_margin", status: "pending" },
];

/** 剥 Rust/Rhai 注释：注释里的名字不算引用面（否则「写一行注释」就能骗过门）。 */
export function stripComments(src) {
  return src.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:"'\\])\/\/[^\n]*/g, "$1");
}

/** mod.rs 的 `const RENAME_MAP` 块＝旧名→新名别名表，属声明面而不是消费面。 */
export function dropRenameMapBlock(lines) {
  const start = lines.findIndex((l) => l.includes("const RENAME_MAP"));
  if (start < 0) return lines;
  let end = lines.length - 1;
  for (let i = start + 1; i < lines.length; i += 1) {
    if (/^\s*\];\s*$/.test(lines[i])) {
      end = i;
      break;
    }
  }
  return lines.filter((_, i) => i < start || i > end);
}

function walk(dir, skip, out) {
  for (const e of readdirSync(dir)) {
    if (skip.includes(e)) continue;
    const p = join(dir, e);
    if (statSync(p).isDirectory()) walk(p, skip, out);
    else if ([".rs", ".rhai", ".ts", ".tsx"].includes(extname(p))) out.push(p);
  }
  return out;
}

/** 引用面语料：整棵 src-tauri/ + src/ 的源码，剥注释、剔掉两份声明面与别名表。 */
export function buildCorpus(fileList) {
  const all = [];
  const rhai = [];
  let fileCount = 0;
  for (const abs of fileList) {
    const rel = abs.slice(ROOT.length + 1).replace(/\\/g, "/");
    if (rel === SEED_REL || rel === PANEL_REL) continue;
    fileCount += 1;
    const raw = readFileSync(abs, "utf8");
    const body = /\.(rs|rhai)$/.test(rel) ? stripComments(raw) : raw;
    const text = rel === MOD_REL ? dropRenameMapBlock(body.split(/\r?\n/)).join("\n") : body;
    all.push(text);
    if (rel.endsWith(".rhai")) rhai.push(text);
  }
  return { corpus: all.join("\n"), rhaiCorpus: rhai.join("\n"), fileCount };
}

export function declaredNames(seedSrc) {
  return [...new Set([...seedSrc.matchAll(/name:\s*"([A-Za-z0-9_]+)"\.into\(\)/g)].map((m) => m[1]))];
}

const camel = (n) => n.replace(/_([a-z])/g, (_, c) => c.toUpperCase());

/** 一个变量是否有引用面：引号形态（snake / camel）在整棵源码里，或 Rhai 里裸标识符。 */
export function hasConsumer(name, { corpus, rhaiCorpus }) {
  if (corpus.includes(`"${name}"`) || corpus.includes(`"${camel(name)}"`)) return true;
  return rhaiCorpus.length > 0 && new RegExp(`(^|[^\\w.])${name}([^\\w]|$)`).test(rhaiCorpus);
}

export function panelExposes(panelSrc, name) {
  return panelSrc.includes(`"${name}"`) || panelSrc.includes(`"${camel(name)}"`);
}

/** 反向读取面：`read_*(vars, "key", 兜底)` 形态的键（现役只有这两处文件）。 */
const READER_REL = [
  "src-tauri/crates/analysis-engine/src/recommender/strategies/reversion.rs",
  "src-tauri/crates/analysis-engine/src/recommender/strategies/trend.rs",
  "src-tauri/crates/analysis-engine/src/backtest_strategy.rs",
];

/**
 * R6 反向登记：**读取侧点的名必须真有人声明**（名实不符＝恒走兜底，面板改了没反应）。
 * 2026-10-08 首跑抓到两条真漂移（`rev_divergence_lookback_days` / `rev_rsi_period_days`，
 * 声明是 `rev_divergence_lookback`=14 / `rev_rsi_period`=6 ⇒ 读取永远不命中、
 * 而兜底值恰好等于默认值，所以从不现形），已当场改名修掉。剩下的逐条登记：
 * `intentional` = 刻意以 Rust 常量为权威、就是不开面板；`pending` = 待裁「要不要做成可调」。
 */
export const READ_KEYS = [
  { key: "trend_ma_tolerance_daily", status: "intentional", cite: "trend.rs:171 注释「消灭每张策略各自手抄一档阈值」⇒ 日基准由 Rust 常量给，各档按 √d 推" },
  { key: "trend_high_tolerance_daily", status: "intentional", cite: "同上（DAILY_HIGH_TOLERANCE + `scale::dev_tolerance`）" },
  ...[
    "trend_short_min_kline_len", "trend_mid_min_kline_len", "trend_long_min_kline_len", "trend_ultra_short_min_kline_len",
    "trend_high_20_period", "trend_high_60_period", "trend_high_ultra_short_period", "trend_high_ultra_short_threshold",
    "trend_ma60_ma250_mult", "trend_ma60_break_mult",
    "value_upper_deviation", "value_lower_deviation", "value_mid_upper_deviation", "value_mid_lower_deviation",
    "value_mid_max_swing", "value_long_upper_deviation", "value_long_lower_deviation", "value_long_max_swing",
    "cap_kline_mom_20_min", "cap_kline_mom_20_max",
    "technical_vol_ratio_min", "technical_ma50_mult", "technical_max_rsi", "technical_ma200_period",
    "technical_long_max_rsi", "technical_long_min_rsi",
  ].map((key) => ({ key, status: "pending" })),
];

/** 抽取读取侧点名的键（剥注释后按 `read_xxx(vars, "键"` 形态），返回 [{key, rel}]。 */
export function collectReadKeys(onlyRel) {
  const out = [];
  for (const rel of onlyRel) {
    const body = stripComments(read(rel));
    for (const m of body.matchAll(/read_[a-z0-9_]*\(\s*\w+,\s*"([a-z0-9_]+)"/g)) out.push({ key: m[1], rel });
  }
  return out;
}

export function checkAll(
  names,
  surf,
  panelSrc,
  exempt = EXEMPT,
  readBaseline = READ_KEYS,
  observed = collectReadKeys(READER_REL),
) {
  if (names.length < MIN_DECLARED) {
    return [`G5 声明面只解析到 ${names.length} 个变量（下限 ${MIN_DECLARED}）⇒ 抽取器失效，不当「无违规」`];
  }
  if (surf.fileCount < MIN_SCANNED_FILES) {
    return [`G5 扫描面只有 ${surf.fileCount} 个文件（下限 ${MIN_SCANNED_FILES}）⇒ 判据没电`];
  }
  const problems = [];
  const registered = new Set(exempt.map((e) => e.name));
  const declared = new Set(names);
  for (const n of names) {
    if (hasConsumer(n, surf) || registered.has(n)) continue;
    const ui = panelExposes(panelSrc, n) ? "，且面板仍渲染控件" : "";
    problems.push(`G1 新增空接线：变量 ${n} 在非声明面零引用${ui} ⇒ 接线，或在 EXEMPT 登记 status`);
  }
  for (const e of exempt) {
    if (!declared.has(e.name)) {
      problems.push(`G3 登记项 ${e.name} 已不在声明面 ⇒ 摘掉这条豁免`);
      continue;
    }
    if (hasConsumer(e.name, surf)) {
      problems.push(`G2 登记项 ${e.name} 现在已有引用面 ⇒ 接线完成，必须摘掉豁免（留着＝面积读数造假）`);
    }
    if (e.status !== "intentional" && e.status !== "pending") {
      problems.push(`G4 ${e.name} 的 status='${e.status}' 不在 {{intentional, pending}} 内`);
    } else if (e.status === "intentional" && !e.cite) {
      problems.push(`G4 ${e.name} 标 intentional 却没 cite ⇒ 「有意不接」必须指到书面裁定`);
    }
  }
  const declaredSet = new Set(names);
  const baseKeys = new Set(readBaseline.map((e) => e.key));
  if (observed.length < 40) {
    problems.push(`G5 读取面只抽到 ${observed.length} 个键（下限 40）⇒ R6 判据没电，不当「无违规」`);
  }
  for (const { key, rel } of observed) {
    if (declaredSet.has(key) || baseKeys.has(key)) continue;
    problems.push(`R6 读取侧点的名无人声明：${rel} 读 "${key}" ⇒ 恒走兜底 ⇒ 补声明，或在 READ_KEYS 登记 status`);
  }
  for (const e of readBaseline) {
    if (declaredSet.has(e.key)) {
      problems.push(`R6 登记项 ${e.key} 现在已有声明 ⇒ 摘掉登记（两处权威并存比没权威更坏）`);
    }
    if (e.status !== "intentional" && e.status !== "pending") {
      problems.push(`R6 ${e.key} 的 status='${e.status}' 不在 {{intentional, pending}} 内`);
    } else if (e.status === "intentional" && !e.cite) {
      problems.push(`R6 ${e.key} 标 intentional 却没 cite ⇒ 「以 Rust 常量为权威」也要指到书面判据`);
    }
  }
  return problems;
}

function deadList(names, surf) {
  return names.filter((n) => !hasConsumer(n, surf));
}

function printArea(names, surf, panelSrc) {
  const dead = deadList(names, surf);
  const ui = dead.filter((n) => panelExposes(panelSrc, n));
  const pending = EXEMPT.filter((e) => e.status === "pending").length;
  const observed = collectReadKeys(READER_REL);
  const undeclared = observed.filter((o) => !new Set(names).has(o.key));
  console.log(
    `声明 ${names.length} ｜ 零引用面 ${dead.length}（已登记 ${dead.filter((n) => EXEMPT.some((e) => e.name === n)).length}）｜ 其中面板仍渲染 ${ui.length} 条`
  );
  console.log(`反向 R6：读取面键 ${observed.length} 个，其中不在声明表 ${undeclared.length} 个（全部已登记：${undeclared.every((o) => READ_KEYS.some((e) => e.key === o.key)) ? "是" : "否"}）`);
  console.log(`登记分列：intentional ${EXEMPT.length - pending} 条、pending ${pending} 条`);
  console.log("⚠ 门绿 ≠ 待修面已清：pending 项仍是「用户改了没人读」的面板控件。");
  return dead;
}

function main() {
  const seedSrc = read(SEED_REL);
  const panelSrc = read(PANEL_REL);
  const names = declaredNames(seedSrc);
  const surf = buildCorpus([
    ...walk(join(ROOT, "src-tauri"), ["target", "node_modules"], []),
    ...walk(join(ROOT, "src"), ["node_modules"], []),
  ]);

  if (process.argv[2] === "--dump") {
    const dead = printArea(names, surf, panelSrc);
    for (const n of dead) {
      const e = EXEMPT.find((x) => x.name === n);
      console.log(`  ${n.padEnd(28)} ${(e ? e.status : "未登记!").padEnd(12)} 面板:${panelExposes(panelSrc, n) ? "有控件" : "无"}`);
    }
    return 0;
  }

  const problems = checkAll(names, surf, panelSrc);
  printArea(names, surf, panelSrc);
  if (problems.length === 0) {
    console.log("✅ 声明变量引用面对账通过");
    return 0;
  }
  for (const p of problems) console.log("❌ " + p);
  console.log(`\n共 ${problems.length} 条`);
  return problems[0].startsWith("G5") ? 2 : 1;
}

function selftest() {
  const seedSrc = read(SEED_REL);
  const panelSrc = read(PANEL_REL);
  const names = declaredNames(seedSrc);
  const surf = buildCorpus([
    ...walk(join(ROOT, "src-tauri"), ["target", "node_modules"], []),
    ...walk(join(ROOT, "src"), ["node_modules"], []),
  ]);
  const base = checkAll(names, surf, panelSrc);
  if (base.length > 0) {
    console.error("❌ 正控失败（现网就该绿）：\n" + base.join("\n"));
    return 1;
  }
  const withExtra = (snippet) => ({ ...surf, corpus: surf.corpus + "\n" + snippet });
  const cases = [
    ["G1 新声明一个零引用变量 ⇒ 点名它", checkAll([...names, "brand_new_dead_var"], surf, panelSrc), "G1 新增空接线：变量 brand_new_dead_var"],
    // ⚠ 负控的**前提样本**必须取「现在仍在登记里」的键（A 批摘掉 9 条后，原先借
    //   `news_limit` / `pos_max_total` 造的三条负控会**恒不命中**＝假绿，同 §一一七(3) 的教训）。
    ["G2 登记项接线了却没摘豁免 ⇒ 红", checkAll(names, withExtra('x.get("val_pb_low");'), panelSrc), "G2 登记项 val_pb_low"],
    [
      "G3 变量删了、豁免还留着 ⇒ 红",
      checkAll(names.filter((n) => n !== "val_pb_high"), surf, panelSrc, EXEMPT),
      "G3 登记项 val_pb_high",
    ],
    [
      "G4 intentional 缺出处 ⇒ 红",
      checkAll(names, surf, panelSrc, [{ name: names[0], status: "intentional" }]),
      `G4 ${names[0]}`,
    ],
    [
      "G4 非法 status ⇒ 红",
      checkAll(names, surf, panelSrc, EXEMPT.map((e) => (e.name === "val_pb_low" ? { name: e.name, status: "wired-later" } : e))),
      "G4 val_pb_low",
    ],
    ["G5 声明面抽取塌陷 ⇒ 红而不是绿", checkAll(["only_one"], surf, panelSrc), "G5 声明面"],
    ["G5 扫描面塌陷 ⇒ 红而不是绿", checkAll(names, { ...surf, fileCount: 10 }, panelSrc), "G5 扫描面"],
    [
      "R6 读取侧点一个没人声明的名 ⇒ 点名该键与文件",
      checkAll(names, surf, panelSrc, EXEMPT, READ_KEYS, [{ key: "totally_undeclared_key", rel: "crates/x.rs" }]),
      'R6 读取侧点的名无人声明：crates/x.rs 读 "totally_undeclared_key"',
    ],
    [
      "R6 登记的键后来有了声明却不摘 ⇒ 红",
      checkAll(names, surf, panelSrc, EXEMPT, [{ key: "rev_divergence_lookback", status: "pending" }], []),
      "R6 登记项 rev_divergence_lookback 现在已有声明",
    ],
    ["剥注释器有效：注释里的名字不算引用面", null, null],
    ["别名表整块被剔：RENAME_MAP 里的新名不算消费面", null, null],
  ];
  let pass = 0;
  for (const [label, got, needle] of cases) {
    if (got === null) {
      const ok =
        label.startsWith("剥注释器")
          ? !stripComments('fn f() {\n  // "news_limit"\n}\n').includes('"news_limit"')
          : !dropRenameMapBlock(['a();', "    const RENAME_MAP: &[(&str, &str)] = &[", '        ("x", "news_limit"),', "    ];", "b();"]).some((l) =>
              l.includes("news_limit"),
            );
      console.log(`${ok ? "✓" : "✗"} ${label}`);
      if (ok) pass += 1;
      continue;
    }
    const hit = got.some((p) => p.includes(needle));
    console.log(`${hit ? "✓" : "✗"} ${label}${hit ? "" : ` ⇒ 期望「${needle}」，实得 ${JSON.stringify(got)}`}`);
    if (hit) pass += 1;
  }
  console.log(`\n负控 ${pass}/${cases.length} 通过`);
  return pass === cases.length ? 0 : 1;
}

const code = process.argv[2] === "--selftest" ? selftest() : main();
if (code !== 0) process.exitCode = code;
