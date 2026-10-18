#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
// 反思词表与落地层级的「三处同域」门 —— 防归因词表与呈现/提示词漂移（2026-10-19，PLAN-mover-coverage 阶段 3）。
//
// ## 为什么要有这道门
// 触发实证（全部取自现网库与当前源码，不是推断）：
//   · `stock_reflections` 实测 80 条 completed 反思里，`failure_modes` 只有 32 条有标注，
//     其中 **23 条单独就是 `["other"]`** —— 兜底值成了主值，看板等于没归因；
//   · 模板（`reflection.md` 与内联 prompt）**早就要求** `implementation_tier`（L1/L2/L3），
//     而 JSON 路径实测 `reflection.implementation_tier` 只有 **1/80** 条有值；
//     面板 `buildAiAssistPrompt` 的缺省写死 `"L1"` ⇒ 79 条「反思没判」被伪装成「判成了参数层」；
//   · `FailureMode` 的 Rust 词表、面板的 `FAILURE_MODE_LABEL_KEYS` 映射、11 份 locale
//     是三份手抄清单，历史上没有任何东西钉住它们 —— 加一个模式要改三处，改漏一处的表现是
//     面板显示 snake_case 原串或直接空白，而不是报错。
// 这三类漂移的共同特征：**没有任何一段代码会失败**，只有数字会变难看。所以由门来失败。
//
// ## 规则
// V1 失败模式三处同域：Rust `FailureMode` 的 `as_str()` 值域 ⇄ 面板 `FAILURE_MODE_LABEL_KEYS`
//    键集必须**互为相等**（少一边即红：少映射＝面板直出原串，多映射＝已退役模式仍占位）；
//    且映射到的每个 i18n 键必须在 11 份 locale 里都存在、非空、且不是中文原文的复制
//    （zh-CN/zh-TW 除外 —— 两者同属中文，同名是正当的）。
// V2 词表必须真进提示词：单一权威是 Rust 的 `catalog_for_prompt()`，注入通道
//    `{{failure_mode_catalog}}` 必须同时出现在内联 prompt 与 `reflection.md`；
//    出现「prompt 里手写了一份模式清单」即红（手抄本不与词表同步）。
// V3 落地层级轴两侧同域：Rust `ReflectionTier` 值域必须全部出现在内联 prompt 与
//    `reflection.md` 的 `implementation_tier` 声明里；反向：面板不得把缺省写成某个
//    具体层级（`tierText = "L1"` 这类伪造一律红）。
// V4 齐备性下限：Rust 模式数、locale 份数、面板映射数、扫描文件数低于现场常量 ⇒ 红。
//    「解析坏了 ⇒ 空集 ⇒ 恒绿」不是通过。
//
// 自证：`--selftest` 五组负控 —— ①Rust 侧摘掉一个模式 ②面板 map 少一个键
//       ③某 locale 少一键 ④面板把缺省写成 "L1" ⑤扫描面清空（下限生效）。
//
// 用法：node scripts/check-reflection-vocabulary.mjs [--selftest] [--dump]

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const FAILURE_MODE_RS = "src-tauri/crates/analysis-engine/src/failure_mode.rs";
const REFLECTION_TIER_RS = "src-tauri/crates/analysis-engine/src/reflection_tier.rs";
const PANEL_TSX = "src/components/stock-analysis/ReflectionPanel.tsx";
const INLINE_PROMPT_RS = "src-tauri/src/commands/stock_analysis_setup/mod.rs";
const REFLECTION_MD = "src-tauri/agency_experts/stock-analysis/reflection.md";
const LOCALES = [
  "ar",
  "de",
  "en-US",
  "es",
  "fr",
  "hi",
  "ja",
  "ko",
  "ru",
  "zh-CN",
  "zh-TW",
];
const I18N_PREFIX = "stockAnalysis.reflection.";

// V4 下限：写明「为什么是这个数」而不是随手拍 ——
// 模式数现值 9（5 类旧词表 + 本例扩的 4 类），真收缩只可能是退役，退役必须同批改三处；
// locale 份数 = 禁区 2 要求的 11；面板映射与模式数同值。
const MIN_FAILURE_MODES = 6;
const MIN_LOCALES = 11;
const MIN_TIERS = 3;

const read = (rel) => fs.readFileSync(path.join(ROOT, rel), "utf8");

/** 剥 Rust 块注释与行注释（词表读的是代码域，注释里的 token 不算值域）。 */
function codeOnly(src) {
  const noBlock = src.replace(/\/\*[\s\S]*?\*\//g, "");
  return noBlock
    .split("\n")
    .filter((ln) => !ln.trimStart().startsWith("//"))
    .join("\n");
}

/**
 * 从 `failure_mode.rs` 抽 as_str() 的值域。
 * 按 `=>` 切臂、只认右值（禁区：邻域法 `grep -B` 会把相邻臂串进来）。
 */
function rustAsStrSet(src, fnName) {
  const start = src.indexOf(`fn ${fnName}(`);
  if (start < 0) throw new Error(`未找到 ${fnName}()`);
  // 函数体到下一个顶层 `}`（缩进 4 的右 brace）为止
  const body = src.slice(start, start + 4000);
  const endIdx = body.indexOf("\n    }");
  const scope = codeOnly(endIdx > 0 ? body.slice(0, endIdx) : body);
  const out = new Map();
  const re = /Self::(\w+)\s*=>\s*"([^"]+)"/g;
  let m;
  while ((m = re.exec(scope)) !== null) out.set(m[1], m[2]);
  return out;
}

/** 抽 `FailureMode::ALL`（或同类常量）里列出的变体名，用于与 as_str 交叉核对。 */
function rustAllSet(src, constName) {
  const start = src.indexOf(`ALL: &'static`);
  if (start < 0) throw new Error(`未找到 ${constName}::ALL`);
  const scope = codeOnly(src.slice(start, start + 1200));
  const endIdx = scope.indexOf("];");
  const body = endIdx > 0 ? scope.slice(0, endIdx) : scope;
  const out = new Set();
  // 前缀既可能是 `Self::` 也可能是 `FailureMode::`（本仓两处写法不同），按路径整体取末段。
  // 只认 `Self::` 会让 ALL 侧读成空集 ⇒ V1a 反过来把 as_str 的每个变体都误报成「未列」。
  const re = /[A-Za-z_]\w*::([A-Za-z_]\w+)/g;
  let m;
  while ((m = re.exec(body)) !== null) out.add(m[1]);
  return out;
}

/** 抽面板 `FAILURE_MODE_LABEL_KEYS` 的键值对（源文件里就是这份手抄清单）。 */
function panelLabelMap(src) {
  const start = src.indexOf("const FAILURE_MODE_LABEL_KEYS");
  if (start < 0) throw new Error("面板未声明 FAILURE_MODE_LABEL_KEYS");
  const body = src.slice(start, src.indexOf("};", start));
  const out = new Map();
  for (const ln of body.split("\n")) {
    const m = ln.match(/^\s*([a-z0-9_]+)\s*:\s*"([^"]+)",?\s*$/);
    if (m) out.set(m[1], m[2]);
  }
  return out;
}

function localeTable() {
  const table = new Map();
  for (const loc of LOCALES) {
    const p = path.join(ROOT, "src/i18n/locales", `${loc}.json`);
    const obj = JSON.parse(fs.readFileSync(p, "utf8"));
    const refl = obj?.stockAnalysis?.reflection ?? {};
    table.set(loc, refl);
  }
  return table;
}

function run() {
  const reds = [];
  const fmSrc = codeOnly(read(FAILURE_MODE_RS));
  const tierSrc = read(REFLECTION_TIER_RS); // 文件不存在即抛错（V4 的「扫描面空」形态）
  const panelSrc = read(PANEL_TSX);
  const promptSrc = read(INLINE_PROMPT_RS);
  const mdSrc = read(REFLECTION_MD);

  const fmAsStr = rustAsStrSet(fmSrc, "as_str");
  const fmAll = rustAllSet(fmSrc, "FailureMode");
  const tierAsStr = rustAsStrSet(tierSrc, "as_str");
  const panelMap = panelLabelMap(panelSrc);
  const locales = localeTable();

  // ── V4 齐备性下限（先跑：解析失效时后面全是空集，必须在这里就红）──
  if (LOCALES.length < MIN_LOCALES)
    reds.push(`V4 locale 份数 ${LOCALES.length} < 下限 ${MIN_LOCALES}（禁区 2 要求 11 种语言）`);
  if (fmAsStr.size < MIN_FAILURE_MODES)
    reds.push(`V4 Rust 失败模式 ${fmAsStr.size} < 下限 ${MIN_FAILURE_MODES}（解析口径失效或词表被整段删除）`);
  if (tierAsStr.size < MIN_TIERS)
    reds.push(`V4 Rust 落地层级 ${tierAsStr.size} < 下限 ${MIN_TIERS}（reflection_tier.rs 的 as_str 没被读到）`);
  if (panelMap.size < MIN_FAILURE_MODES)
    reds.push(`V4 面板映射条目 ${panelMap.size} < 下限 ${MIN_FAILURE_MODES}（map 被清空 ⇒ V1 会恒绿）`);

  // ── V1a Rust 内部自洽：ALL 列出的变体与 as_str 覆盖的必须同集 ──
  for (const v of fmAll) if (!fmAsStr.has(v)) reds.push(`V1a 词表 '${v}' 在 ALL 里但 as_str 无映射（看板会漏一格）`);
  for (const v of fmAsStr.keys()) if (!fmAll.has(v)) reds.push(`V1a as_str 有 '${v}' 但 ALL 未列（模板清单不显示它）`);

  // ── V1b Rust ⇄ 面板 ⇄ locale 三处同域 ──
  const modes = [...fmAsStr.values()];
  for (const mode of modes) {
    if (!panelMap.has(mode))
      reds.push(`V1 失败模式 '${mode}' 无面板映射 ⇒ 看板直出 snake_case 原串（词表与呈现漂移）`);
  }
  for (const mode of panelMap.keys()) {
    if (!modes.includes(mode))
      reds.push(`V1 面板映射含 '${mode}' 但 Rust 词表已无 ⇒ 退役没退干净（占位假格）`);
  }
  const zhCn = locales.get("zh-CN") || {};
  for (const [, key] of panelMap) {
    const full = I18N_PREFIX + key;
    for (const loc of LOCALES) {
      const table = locales.get(loc) || {};
      const v = table[key];
      if (typeof v !== "string" || v.trim() === "") {
        reds.push(`V1 locale ${loc} 缺译名 ${full}（该语言的看板此格显示键名或空白）`);
        continue;
      }
      // 反抄写：非中文语言的值不得就是 zh-CN 原文（中文两语除外 —— 同名是正当的）
      if (!loc.startsWith("zh") && v === zhCn[key])
        reds.push(`V1 locale ${loc} 的 ${full} 直接抄了 zh-CN 原文（缺真翻译）`);
    }
  }

  // ── V2 注入通道：词表只能靠 Rust 单一权威渲染进 prompt，prompt 里不得另抄一份 ──
  const catalogHits = [
    ["内联 prompt", promptSrc.includes("{{failure_mode_catalog}}")],
    ["reflection.md", mdSrc.includes("{{failure_mode_catalog}}")],
  ];
  for (const [who, ok] of catalogHits) {
    if (!ok) reds.push(`V2 ${who} 未注入 {{failure_mode_catalog}} ⇒ 词表扩充进不了模型视野`);
  }
  // 手写清单探测：prompt 侧把某个具体模式名当**字面量选项**写死（应当只写占位符）。
  // 必须带引号作数 —— 裸子串会被英文里的 `other`（`#[serde(other)]`、`other_horizon`）命中，
  // 那种假阳会把规则本身训成「忽略这条报错」。
  for (const mode of modes) {
    if (promptSrc.includes(`"${mode}"`))
      reds.push(`V2 '${mode}' 以内联 prompt 的字面量出现 ⇒ 存在第二份手抄词表（改一处不动另一处）`);
  }
  // reflection.md 允许保留**一个**示例值（模板示例本身要具体才看得懂）；多于一个即
  // 从「举例」变成「第二份清单」，与 Rust 词表必然漂移。
  const mdLiterals = modes.filter((m) => mdSrc.includes(`"${m}"`));
  if (mdLiterals.length > 1)
    reds.push(
      `V2 reflection.md 以字面量出现 ${mdLiterals.length} 个模式名（>1 即第二份清单）：${mdLiterals.join(", ")}`,
    );

  // ── V3 落地层级轴：Rust ⇄ 两处 prompt 声明 ⇄ 面板不得伪造缺省 ──
  // 内联 prompt 在 Rust 源里是转义引号（`\"implementation_tier\": \"L1 | L2 | L3\"`），
  // 因此按「同一行里同时出现轴名与该值」判，不按引号形态判。
  const promptTierLines = promptSrc.split("\n").filter((ln) => ln.includes("implementation_tier"));
  if (promptTierLines.length === 0)
    reds.push("V3 内联 prompt 完全没有 implementation_tier 声明（轴已退役？同步删本门这条规则再撤）");
  for (const tier of tierAsStr.values()) {
    if (!promptTierLines.some((ln) => ln.includes(tier)))
      reds.push(`V3 内联 prompt 的 implementation_tier 声明行里没有 ${tier}`);
    if (!new RegExp(`\\*\\*${tier}\\*\\*`).test(mdSrc))
      reds.push(`V3 reflection.md 的层级表未声明 ${tier}（表里没有该行）`);
  }
  if (/let tierText\s*=\s*"L[123]"/.test(panelSrc))
    reds.push("V3 面板把 implementation_tier 的缺省写成具体层级 ⇒ 「反思没判」被伪装成某一层（伪造读数）");

  return {
    reds,
    modes,
    tiers: [...tierAsStr.values()],
    panelMapSize: panelMap.size,
  };
}

function classify(res, baseline) {
  const newReds = [];
  const hit = new Set();
  for (const r of res.reds) {
    const b = baseline.find((x) => r.includes(x.token));
    if (b) hit.add(b);
    else newReds.push(r);
  }
  return { newReds, stale: baseline.filter((b) => !hit.has(b)) };
}

const BASELINE = [];

function main() {
  const args = process.argv.slice(2);
  const dump = args.includes("--dump");
  if (args.includes("--selftest")) return selftest();

  let res;
  try {
    res = run();
  } catch (e) {
    console.error(`❌ 门自身跑不起来（扫描面/解析口径问题，不是被测代码问题）：${e.message}`);
    process.exit(1);
  }
  if (dump)
    console.log(
      `Rust 失败模式（${res.modes.length}）：${res.modes.join(", ")}\nRust 层级（${res.tiers.length}）：${res.tiers.join(", ")}\n面板映射条目：${res.panelMapSize}`,
    );
  const cls = classify(res, BASELINE);
  for (const b of cls.stale)
    console.log(`   ▶ '${b.token}' 已不再命中 ⇒ 该面已收口，请把此行从 BASELINE 摘掉`);
  if (cls.newReds.length > 0) {
    console.error(`❌ 反思词表/层级对账违规 ${cls.newReds.length} 条：`);
    for (const r of cls.newReds) console.error("  - " + r);
    process.exit(1);
  }
  console.log(
    `✅ 反思词表三处同域（Rust ${res.modes.length} 模式 ⇄ 面板 ${res.panelMapSize} 映射 ⇄ ${LOCALES.length} 语言），层级轴 ${res.tiers.length} 值两侧一致，面板缺省未伪造`,
  );
}

function selftest() {
  const EN_P = path.join("src/i18n/locales", "en-US.json");
  const ZH_P = path.join("src/i18n/locales", "zh-CN.json");
  // 快照先行：四个被改文件全部先读进内存，restore() 无条件逐个回写。
  // （教训同形：`write(p, x)` 的实参必须是**已建好的变量**，不能是「读自己」的表达式 ——
  //  那会把文件截成 0 字节。）
  const orig = {
    panel: read(PANEL_TSX),
    zh: read(ZH_P),
    en: read(EN_P),
    fm: read(FAILURE_MODE_RS),
  };
  const restore = () => {
    const pairs = [
      [PANEL_TSX, orig.panel],
      [ZH_P, orig.zh],
      [EN_P, orig.en],
      [FAILURE_MODE_RS, orig.fm],
    ];
    for (const [rel, txt] of pairs) {
      if (txt && txt.length > 100) fs.writeFileSync(path.join(ROOT, rel), txt, "utf8");
    }
  };
  const probe = (rel, txt) => {
    fs.writeFileSync(path.join(ROOT, rel), txt, "utf8");
    return run().reds;
  };
  const cases = [];
  try {
    const base = run().reds;
    cases.push(["基线（未改动）红条数", base.length, undefined]);

    // ① Rust 侧改一个模式名 ⇒ 期望 V1「面板映射含已退役模式」命中（而不是全绿）
    const drop = orig.fm.replace(`Self::Other => "other",`, `Self::Other => "other_renamed",`);
    let r = probe(FAILURE_MODE_RS, drop);
    cases.push(["①改 Rust 模式名后红条数", r.length, r.some((x) => x.includes("面板映射含"))]);
    restore();

    // ② 面板 map 少一个键 ⇒ 期望 V1「无面板映射」命中
    const noMap = orig.panel.replace(
      `  theme_rotation_missed: "failureModeThemeRotationMissed",\n`,
      "",
    );
    r = probe(PANEL_TSX, noMap);
    cases.push(["②面板少映射后红条数", r.length, r.some((x) => x.includes("无面板映射"))]);
    restore();

    // ③ 某 locale 少一键 ⇒ 期望 V1「缺译名」命中
    const zhObj = JSON.parse(orig.zh);
    delete zhObj.stockAnalysis.reflection.failureModeThemeRotationMissed;
    r = probe(ZH_P, JSON.stringify(zhObj, null, 2) + "\n");
    cases.push(["③locale 缺键后红条数", r.length, r.some((x) => x.includes("缺译名"))]);
    restore();

    // ④ 面板伪造缺省 ⇒ 期望 V3 命中
    const fake = orig.panel.replace(
      'let tierText = "(反思未声明 implementation_tier',
      'let tierText = "L1"; //x "(反思未声明 implementation_tier',
    );
    r = probe(PANEL_TSX, fake);
    cases.push(["④面板写死 L1 后红条数", r.length, r.some((x) => x.includes("伪装成某一层"))]);
    restore();

    // ⑤ 抄写检测：en-US 的值改成 zh-CN 原文 ⇒ 期望「直接抄了 zh-CN 原文」命中
    const enObj = JSON.parse(orig.en);
    enObj.stockAnalysis.reflection.failureModeOther = JSON.parse(
      orig.zh,
    ).stockAnalysis.reflection.failureModeOther;
    r = probe(EN_P, JSON.stringify(enObj, null, 2) + "\n");
    cases.push(["⑤英文抄中文后红条数", r.length, r.some((x) => x.includes("抄了 zh-CN"))]);
    restore();
  } finally {
    restore();
  }
  let ok = true;
  for (const [name, n, expectHit] of cases) {
    console.log(`  ${name}: ${n} 条红${expectHit === undefined ? "" : expectHit ? "（负控命中）" : "（负控未命中）"}`);
    if (expectHit === false) ok = false;
  }
  const after = run().reds.length;
  console.log(`  复原后红条数回到基线：${after}`);
  if (after !== cases[0][1] || !ok) {
    console.error("❌ 自测未通过（负控没命中或复原失败）");
    process.exit(1);
  }
  console.log("✅ 自测通过（5 项断言：每类破坏只让对应那条红，且复原回基线）");
}

main();
