// 面板 9 条变量「Rust 内部读变量表」落地的**定向门**（2026-10-08 A 批接线）
//
// ── 为什么必须有这道门（面积门 `check-variable-consumer-registry.mjs` 查不出这些）──
// 那 9 条的接法不是 v137 那种「工具参数」，而是「下层 crate 读进程内变量表快照」
// （端口 `harness::panel_variables`，装入口 `init/panel_variables.rs`）。于是新增三类
// **只有定向门能发现、编译门一条都查不出**的失效：
//   ① **默认值漂移**：种子/面板那格是手抄的。落点的回落值（`ScoreBands::default()` 的 30/70、
//      `PositionLimits::default()` 的 20/10/40、`PeBands::default()` 的 15/50、
//      `monitor.rs` 的 `RwLock::new(30)`、`DEFAULT_NEWS_LIMIT=30`）与它不等 ⇒ 接线当场改
//      现网数值，而 A 批的入场券正是「逐位不变」。
//   ② **函数没接上**：变量名在表里、`with_panel_overlay` 也写了，但生产路径没人调用那个
//      构造函数 ⇒ 一切看起来都对，面板仍是空接线（`start_with_config` 此前就是零调用者的正门）。
//   ③ **第二权威复活**：落点旁边又写回一份字面量。本批真出现过两处 —— 回放链
//      `stock_analysis.rs` 的 PE 20/40 与 `verify_catalysts_impl` 写死的 50 条新闻。
//      只改一处必留另一处 ⇒ 这条判据是**双向**的（不许再出现字面量，且必须引用落点）。
//
// ── 五条判据 ──
//   P1 三面默认值对账：`seed_variables.rs` 声明值 ≡ 落点回落值；面板有本地 `b()` 兜底时还必须 ≡ 它
//      （三面而不两面：`signal_rsi_*` 两条**面板没有控件**，只比两侧会把「面板改过、种子没改」漏掉）
//   P2 消费者存在：每条的落点文件里有引号形态的键，且落点构造函数在**生产调用形态**上被引用 ≥ 规定处数
//      （数的是 `client.get_news(stock_code, panel_news_limit())` 这种整条调用，不是函数名出现次数 ——
//       后者会被注释与测试满足，起不到「真的被调用」的作用。`.rs` 面统一先剥注释、再剥
//       `#[cfg(test)] mod`：A 批给每条落点配的行为锁测试**就在同一个文件里**手抄了键名与调用形态，
//       不剥就是「测试替自己作证」—— 生产表被删、测试里还剩五处 ⇒ 门照样绿。实测这条就是把
//       负控「键被改名」逼成失败的那条（补剥离前 9/10，补后 13/13）。）
//   P3 单一权威：回放链 PE 与新闻条数不许再出现手抄字面量，且必须引用落点
//   P4 面积自证：落地清单恰 9 条（=A 批全量），且每条都仍在声明面（退役一条要同步摘这里）
//   P5 解析面自证：任一侧一条都取不到 ⇒ 判**红**（「判据没电」不等于「全绿」）
//
// ── 本门**看不见**的三件事（读数别过度解读，2026-10-08 验收时记下的实测盲区）──
//   ① 「调用存在」≠「调用可达」：P2 数的是落点函数在**生产文本**里的出现处数。若某个消费者
//      本身没有活调用方（例：`trading.rs` 的 `validate_trade_with_config` 全仓零调用者），
//      门仍绿而面板仍是死的。本批九条的可达性是靠人工沿 `rhai 脚本 → register_fn → 落点`
//      这条链逐条核过的（见 A 批验收记录），不是门给的保证。
//   ② 装配时机：`monitor_poll_interval_secs` 只在**启动装配**时读一次快照
//      （`services.rs` 的 `start_with_config`），而 `set_poll_interval_secs` 至今零调用者
//      ⇒ 面板改了要**重启才生效**。门只要求「至少装配一次」，看不出这个窗口。
//   ③ 参数优先级：`news_limit` 在工具入口是「节点/LLM 显式传的 `limit` 优先」（`news_limit_from_arguments`）
//      ⇒ LLM 自带 limit 的那次调用不受面板值支配。这是刻意保留的今天形态，不是漏洞，但会让
//      用户觉得「改了没反应」。
//
// ── 用法 ──
//   node scripts/check-panel-var-landing.mjs             # 门禁（0/1）
//   node scripts/check-panel-var-landing.mjs --selftest   # 负控全集（条数由 cases.length 现算），每条必须真能红
//   node scripts/check-panel-var-landing.mjs --dump       # 打印三面逐条读数 + 每条的生产调用处数
//
// 退出码：0 通过 ｜ 1 违规（含判据没电）

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const read = (rel) => readFileSync(resolve(ROOT, rel), "utf8");

const PANEL_REL = "src/components/settings/StockAnalysisConfigPanel.tsx";
const SCORING_REL = "src-tauri/crates/astock-data/src/scoring.rs";
const POS_REL = "src-tauri/crates/analysis-engine/src/position_limits.rs";
const MONITOR_REL = "src-tauri/crates/analysis-engine/src/monitor.rs";
const MCP_REL = "src-tauri/crates/astock-data/src/mcp_tools.rs";
const FORMULA_REL = "src-tauri/crates/analysis-engine/src/portfolio_formula.rs";
const TRADING_REL = "src-tauri/crates/analysis-engine/src/trading.rs";
const SERVICES_REL = "src-tauri/src/init/services.rs";
const REPLAY_REL = "src-tauri/src/commands/stock_analysis.rs";
const SEED_REL = "src-tauri/src/commands/stock_analysis_setup/seed_variables.rs";

/**
 * 落地面（本批的**唯一清单**，9 条 = A 批全量）。
 * 三侧的期望值一律**从源码现取**，门里不写死数字（写死了就变成第四份手抄）。
 * - `rust`：落点回落默认值的现取处（文件 + 锚定正则，第 1 组就是那个数）
 * - `key` ：变量名必须以 `"名"` 形态出现的落点文件
 * - `calls`：生产调用形态的最少处数
 */
export const LANDED = [
  {
    name: "signal_rsi_oversold",
    rust: { rel: SCORING_REL, re: /impl Default for ScoreBands[\s\S]*?^\s*rsi_oversold:\s*(-?[0-9.]+),/m },
    key: { rel: SCORING_REL, table: "RSI_BAND_VARS" },
    calls: [
      { rel: SCORING_REL, needle: "&ScoreBands::panel_effective()", min: 1 },
      { rel: SCORING_REL, needle: "ScoreBands::scaled_for_panel(profile)", min: 1 },
      { rel: MCP_REL, needle: "ScoreBands::scaled_for_panel(&profile)", min: 1 },
    ],
  },
  {
    // 与 oversold 共用同一次覆盖调用 ⇒ 调用处数已由上一条把关，这里只查键与默认值。
    name: "signal_rsi_overbought",
    rust: { rel: SCORING_REL, re: /impl Default for ScoreBands[\s\S]*?^\s*rsi_overbought:\s*(-?[0-9.]+),/m },
    key: { rel: SCORING_REL, table: "RSI_BAND_VARS" },
    calls: [],
  },
  {
    name: "pos_max_single_pct",
    rust: { rel: POS_REL, re: /impl Default for PositionLimits[\s\S]*?max_single_stock_pct:\s*(-?[0-9.]+)/ },
    key: { rel: POS_REL, table: "POSITION_LIMIT_VARS" },
    calls: [
      { rel: FORMULA_REL, needle: "PositionLimits::panel_effective()", min: 2 },
      { rel: TRADING_REL, needle: "PositionLimits::panel_effective()", min: 1 },
      { rel: REPLAY_REL, needle: "PositionLimits::panel_effective()", min: 3 },
    ],
  },
  {
    name: "pos_max_total",
    rust: { rel: POS_REL, re: /impl Default for PositionLimits[\s\S]*?max_total_positions:\s*(-?[0-9.]+)/ },
    key: { rel: POS_REL, table: "POSITION_LIMIT_VARS" },
    calls: [],
  },
  {
    name: "pos_max_sector_pct",
    rust: { rel: POS_REL, re: /impl Default for PositionLimits[\s\S]*?max_sector_exposure_pct:\s*(-?[0-9.]+)/ },
    key: { rel: POS_REL, table: "POSITION_LIMIT_VARS" },
    calls: [],
  },
  {
    name: "val_pe_low",
    rust: { rel: SCORING_REL, re: /impl Default for PeBands[\s\S]*?low:\s*(-?[0-9.]+)/ },
    key: { rel: SCORING_REL, table: "PE_BAND_VARS" },
    calls: [
      { rel: SCORING_REL, needle: "&PeBands::panel_effective()", min: 1 },
      // 回放链的第二权威必须改成读同一个来源（P3 还另外查「不许再出现 20/40」）。
      { rel: REPLAY_REL, needle: "PeBands::panel_effective()", min: 1 },
    ],
  },
  {
    name: "val_pe_high",
    rust: { rel: SCORING_REL, re: /impl Default for PeBands[\s\S]*?high:\s*(-?[0-9.]+)/ },
    key: { rel: SCORING_REL, table: "PE_BAND_VARS" },
    calls: [],
  },
  {
    name: "monitor_poll_interval_secs",
    rust: { rel: MONITOR_REL, re: /poll_interval_secs:\s*RwLock::new\(\s*(-?[0-9.]+)\s*\)/ },
    key: { rel: SERVICES_REL, table: "start_realtime_monitor 装配点" },
    calls: [
      // 正门 `start_with_config` 此前**全仓零调用者** ⇒ 必须至少有一次真装配。
      { rel: SERVICES_REL, needle: "start_with_config(", min: 1 },
    ],
  },
  {
    name: "news_limit",
    rust: { rel: MCP_REL, re: /const DEFAULT_NEWS_LIMIT:\s*u32\s*=\s*(-?[0-9.]+)/ },
    key: { rel: MCP_REL, table: "panel_news_limit" },
    calls: [
      // 三处内部取数点 + 两处工具入口（个股新闻 / 政策新闻），按**整条调用形态**数。
      { rel: MCP_REL, needle: "client.get_news(stock_code, panel_news_limit())", min: 3 },
      { rel: MCP_REL, needle: "news_limit_from_arguments(arguments)", min: 2 },
    ],
  },
];

/**
 * P3 单一权威：这些**字面量形态**一旦出现就红（两侧各查一遍，防「改了一处、留了另一处」）。
 * `absent` = 不许出现的源码；`present` = 必须出现的源码（同一判据的两半）。
 */
export const SINGLE_AUTHORITY = [
  {
    why: "回放链的 PE 分档必须读 `PeBands`，不许再抄 20/40",
    rel: REPLAY_REL,
    absent: ["*pe < 20.0", "*pe > 40.0"],
    present: ["PeBands::panel_effective()"],
  },
  {
    why: "新闻条数只允许一个来源，不许再写死 30 / 50",
    rel: MCP_REL,
    absent: [
      "get_news(stock_code, 30)",
      "get_news(stock_code, 50)",
      'arguments["limit"].as_u64().unwrap_or(30)',
    ],
    present: ["client.get_news(stock_code, panel_news_limit())"],
  },
];

/**
 * needle → 排版无关的正则：needle 里的每处空白放宽成 `\s+`，其余字符按字面量转义。
 *
 * 为什么必须这样比：needle 抄的是**源码形态**（`client.get_news(stock_code, panel_news_limit())`），
 * 而 rustfmt / dprint 会按行宽把一条调用折行。实测：`cargo fmt --all` 跑过一次之后，
 * `POSITION_LIMIT_VARS` 里 `("pos_max_total", |v| …)` 被拆成竖排三行 ⇒ 两条负控的字符串替换
 * 变成空操作、当场假红（本门一度只有 11/13）。判据绑排版 = 迟早逼人关门。
 * 放宽空白不放宽语义：仍是「整条调用形态」，只是不再管它被折成几行。
 */
function needleRegExp(needle) {
  const escaped = needle.replace(/[.*+?^${}()|[\]\\]/g, "\\$&").replace(/\s+/g, "\\s+");
  return new RegExp(escaped, "g");
}

/** 数 needle 出现几次（排版无关，见 [`needleRegExp`]）。 */
function count(hay, needle) {
  return (hay.match(needleRegExp(needle)) || []).length;
}

/** needle 是否出现（排版无关）。 */
function hasText(hay, needle) {
  return needleRegExp(needle).test(hay);
}

/**
 * 把源码里的某一段字面量换成另一段（负控用；同样排版无关，见 [`needleRegExp`]）。
 * 用函数形式替换，避免 replacement 里的 `$` 被当特殊字符。
 */
function patchText(src, from, to) {
  return src.replace(needleRegExp(from), () => to);
}

/**
 * 剥 Rust 注释（形态与 `check-variable-consumer-registry.mjs` 的 `stripComments` 逐字一致）。
 *
 * 为什么必须剥：本门三条判据数的是**源码里的字面量与调用**，注释里出现
 * 「`*pe < 20.0`」或「调用 `PositionLimits::panel_effective()`」不该算数 ——
 * 否则回放链那条讲历史缺陷的注释会把 P3 顶成常红，而 P2 的调用处数也能靠注释凑够。
 * 同一条理由见面积门头上的「注释里的名字不算引用面（否则写一行注释就能骗过门）」。
 */
export function stripComments(src) {
  return src.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:"'\\])\/\/[^\n]*/g, "$1");
}

/**
 * 剥 `#[cfg(test)] mod …{…}` 整块（大括号配平；形态不认识 / 括号不闭合 ⇒ **保守不剥**）。
 *
 * 为什么必须有它（2026-10-08 补，本门的自证负控把它逼出来的）：P2 问的是
 * 「生产路径真的读这个键吗」。而 A 批给每条落点都配了行为锁测试，那些测试**就在同一个文件里**
 * 手抄了键名与调用形态（`("pos_max_total", serde_json::json!(3))`）。于是「测试替自己作证」：
 * 把生产表里的键删掉、测试里还剩五处 ⇒ 判据照样绿，而现网已经没人读它。
 * 负控「把落点表里的键改名 ⇒ 必须红」在补这个剥离之前**就是红的失败方**（实测 9/10）。
 * 注释不算数、测试也不算数 —— 只有生产代码算数，这才是「接上了」的定义。
 */
export function stripTestModules(src) {
  const attr = "#[cfg(test)]";
  const modHead = /^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+[A-Za-z_][A-Za-z0-9_]*\s*\{/;
  let out = src;
  // 从前往后逐个处理；每次剥离后游标停在属性处继续找下一个（一个文件可有多块）。
  for (let pos = 0; (pos = out.indexOf(attr, pos)) !== -1; ) {
    const m = modHead.exec(out.slice(pos + attr.length));
    if (!m) {
      pos += attr.length; // 不是 `mod`（如贴在 use/impl 上）⇒ 跳过，宁漏剥不错删生产代码
      continue;
    }
    const braceStart = pos + attr.length + m.index + m[0].length - 1;
    let depth = 0;
    let end = -1;
    for (let j = braceStart; j < out.length; j += 1) {
      if (out[j] === "{") depth += 1;
      else if (out[j] === "}" && (depth -= 1) === 0) {
        end = j;
        break;
      }
    }
    if (end === -1) {
      pos += attr.length; // 括号不闭合 ⇒ 同样保守不剥（本门只读真实文件，出现即说明形态超预期）
      continue;
    }
    out = out.slice(0, pos) + out.slice(end + 1);
  }
  return out;
}

/**
 * 声明面 / 落点面的统一取源口径：`.rs` 先剥注释再剥 test 模块，其余原样。
 *
 * `.tsx` 刻意不剥（与 `check-variable-consumer-registry.mjs` 同一口径）：TS 里的 `//`
 * 会出现在 JSX 文本与正则字面量里，误剥的代价比漏剥大；而面板面只有 `b("名", 数字,` 一种形态要抽。
 */
function stripForLint(rel, raw) {
  if (!rel.endsWith(".rs")) return raw;
  return stripTestModules(stripComments(raw));
}

/** 面板那侧的 `b("<名>", <数字>, …)` ⇒ { 名: 值 }；一条都没取到（含源缺失）⇒ null（判据没电）。 */
export function panelDefaults(src) {
  if (typeof src !== "string") return null;
  const out = {};
  const re = /\bb\("([a-z_]+)",\s*(-?[0-9.]+)\s*,/g;
  let m;
  while ((m = re.exec(src)) !== null) out[m[1]] = Number(m[2]);
  return Object.keys(out).length > 0 ? out : null;
}

/** `seed_variables.rs` 的声明面（名字 + 默认值）；源缺失或一条都取不到 ⇒ null。P4 与 P1 都要用它。 */
export function seedDefaults(src) {
  if (typeof src !== "string") return null;
  const out = {};
  const re = /name:\s*"([A-Za-z0-9_]+)"\.into\(\),[\s\S]{0,160}?value:\s*serde_json::json!\(\s*(-?[0-9.]+)\s*\)/g;
  let m;
  while ((m = re.exec(src)) !== null) {
    if (!(m[1] in out)) out[m[1]] = Number(m[2]);
  }
  return Object.keys(out).length > 0 ? out : null;
}

/** 落点的回落默认值（现取，不写在门里）；取不到 ⇒ undefined。 */
function rustDefault(entry, srcOf) {
  const src = srcOf(entry.rust.rel);
  if (src === undefined) return undefined; // 文件读不到 ⇒ 交给 P5 报「判据没电」
  const m = entry.rust.re.exec(src);
  return m ? Number(m[1]) : undefined;
}

/**
 * 五条判据 + 两环自证一起跑，返回问题串数组（空 = 通过）。
 * 四个入参可替换（自证用）：`srcOf(rel)` 返回该文件源码，`undefined` 表示读不到。
 */
export function checkAll(panelSrc, srcOf, seedSrc, landed = LANDED, authority = SINGLE_AUTHORITY) {
  const problems = [];
  const panel = panelDefaults(panelSrc);
  const seed = seedDefaults(seedSrc);
  if (!panel) problems.push('P5 判据没电：面板 b("<名>", <数字>, …) 一条都没取到');
  if (!seed) problems.push("P5 判据没电：seed_variables.rs 的 name+value 一条都没取到");
  // P4 面积自证：本门只管 A 批那 9 条，多一条少一条都要当场说清（悄悄删条目＝门失去读数）。
  if (landed.length !== 9) {
    problems.push(`P4 落地清单是 ${landed.length} 条（应为 9 条=A 批全量）⇒ 改判据前先讲清为什么`);
  }

  for (const e of landed) {
    if (!seed || !(e.name in seed)) {
      problems.push(`P4 ${e.name} 已不在声明面（或声明侧抽取失效）⇒ 从本门清单摘掉，别留指向不存在的键`);
    }
    // ── P1 三面默认值对账 ──
    const rust = rustDefault(e, srcOf);
    if (rust === undefined) {
      problems.push(`P5 判据没电：${e.rust.rel} 里取不到 ${e.name} 的回落默认（锚定正则失效或被改名）`);
    }
    if (seed && rust !== undefined && e.name in seed && seed[e.name] !== rust) {
      problems.push(
        `P1 ${e.name}：种子声明 ${seed[e.name]} ≠ 落点回落 ${rust} ⇒ 接线当场会改现网数值（A 批的入场券是逐位不变）`
      );
    }
    // 面板有本地兜底时才比（`signal_rsi_*` 两条**面板没有控件**，不是漏接）。
    if (panel && seed && e.name in panel && e.name in seed && panel[e.name] !== seed[e.name]) {
      problems.push(
        `P1 ${e.name}：面板本地兜底 ${panel[e.name]} ≠ 种子声明 ${seed[e.name]} ⇒ 模板变量表为空时面板会把种子值顶掉`
      );
    }
    // ── P2 消费者存在 ──
    const landingSrc = srcOf(e.key.rel);
    if (landingSrc === undefined) {
      problems.push(`P5 判据没电：落点文件 ${e.key.rel} 读不到`);
    } else if (!landingSrc.includes(`"${e.name}"`)) {
      problems.push(`P2 ${e.name} 的键没出现在落点 ${e.key.rel}（${e.key.table} 表被改名或删掉？）⇒ 面板对它仍是空接线`);
    }
    for (const c of e.calls) {
      const src = srcOf(c.rel);
      const got = src === undefined ? 0 : count(src, c.needle);
      if (got < c.min) {
        problems.push(
          `P2 ${c.rel} 里 ${c.needle} 只有 ${got} 处（应 ≥${c.min}）⇒ 落点函数写了但生产路径没调用，等于没接`
        );
      }
    }
  }

  // ── P3 单一权威（不许复活，且必须引用落点）──
  for (const a of authority) {
    const src = srcOf(a.rel);
    if (src === undefined) {
      problems.push(`P5 判据没电：${a.rel} 读不到（P3「${a.why}」无法执行）`);
      continue;
    }
    for (const bad of a.absent) {
      if (hasText(src, bad)) {
        problems.push(`P3 ${a.rel} 又出现手抄字面量 ${bad} ⇒ ${a.why}（两处权威并存比没权威更坏）`);
      }
    }
    for (const need of a.present) {
      if (!hasText(src, need)) {
        problems.push(`P3 ${a.rel} 不再引用 ${need} ⇒ ${a.why}`);
      }
    }
  }
  return problems;
}

/** 一次读齐所有被引用文件（自证与主流程共用，避免同一文件读多遍）。`.rs` 剥注释 + 剥 test 模块。 */
function sourceCache(relList) {
  const cache = new Map();
  for (const rel of relList) {
    try {
      cache.set(rel, stripForLint(rel, read(rel)));
    } catch {
      // 读不到 ⇒ 存 undefined，让判据报「没电」而不是崩在读取上
    }
  }
  return (rel) => cache.get(rel);
}

function allRels() {
  const rels = new Set([PANEL_REL, SEED_REL]);
  for (const e of LANDED) {
    rels.add(e.rust.rel);
    rels.add(e.key.rel);
    for (const c of e.calls) rels.add(c.rel);
  }
  for (const a of SINGLE_AUTHORITY) rels.add(a.rel);
  return [...rels];
}

function report(problems, srcOf) {
  if (problems.length === 0) {
    const seed = seedDefaults(srcOf(SEED_REL));
    const panel = panelDefaults(srcOf(PANEL_REL));
    const rows = LANDED.map(
      (e) => `${e.name} 种子${seed[e.name]}/落点${rustDefault(e, srcOf)}${e.name in panel ? `/面板${panel[e.name]}` : "/面板无控件"}`
    );
    console.log("✅ 面板 9 条变量落地：三面默认值对账 + 消费者存在 + 单一权威 + 面积自证 全部通过");
    console.log(`   逐字读数：${rows.join("  ")}`);
    // 面积读数**逐条点名到文件**（只给总数就退回「一道会说谎的绿」）：每条的落点文件 + 生产调用处数。
    for (const e of LANDED) {
      const at = e.key.rel.replace(/^src-tauri\//, "");
      const calls = e.calls.length === 0
        ? "（与同表兄弟共用一次覆盖调用）"
        : e.calls.map((c) => `≥${c.min}@${c.rel.replace(/^src-tauri\//, "")}`).join(" ");
      console.log(`   ${e.name.padEnd(26)} 键在 ${at}  ${calls}`);
    }
    return 0;
  }
  for (const p of problems) console.log("❌ " + p);
  console.log(`\n共 ${problems.length} 条`);
  return 1;
}

function dump(srcOf) {
  const panel = panelDefaults(srcOf(PANEL_REL));
  const seed = seedDefaults(srcOf(SEED_REL));
  console.log(
    `声明面 ${Object.keys(seed || {}).length} 条取值 ｜ 面板 ${Object.keys(panel || {}).length} 条本地兜底 ｜ 落地清单 ${LANDED.length} 条`
  );
  for (const e of LANDED) {
    const keys = srcOf(e.key.rel) ?? "";
    const calls = e.calls.map((c) => `${c.needle}@${count(srcOf(c.rel) ?? "", c.needle)}/${c.min}`).join(", ");
    console.log(
      `  ${e.name.padEnd(26)} 种子:${seed?.[e.name]}  落点:${rustDefault(e, srcOf)}  面板:${e.name in (panel || {}) ? panel[e.name] : "无控件"}  键在 ${e.key.rel}:${keys.includes(`"${e.name}"`) ? "有" : "无"}  ${calls}`
    );
  }
  for (const a of SINGLE_AUTHORITY) {
    const src = srcOf(a.rel) ?? "";
    console.log(
      `  单一权威 ${a.rel}：手抄残留 ${a.absent.filter((x) => hasText(src, x)).length} 处，落点引用 ${a.present.filter((x) => hasText(src, x)).length}/${a.present.length}`
    );
  }
}

function selftest() {
  const srcOf = sourceCache(allRels());
  // 与门禁模式**同一份**取源（`.rs` 已剥注释与 test 模块），否则负控验的是另一套代码。
  const panelSrc = srcOf(PANEL_REL);
  const seedSrc = srcOf(SEED_REL);
  const clean = checkAll(panelSrc, srcOf, seedSrc);
  if (clean.length > 0) {
    console.error("❌ 正控失败（现网就该绿）：\n" + clean.join("\n"));
    return 1;
  }
  /** 造一个「只把某个文件换成 patch 后源码」的 srcOf（负控共用）。 */
  const withPatch = (rel, fn) => {
    const cache = new Map();
    for (const r of allRels()) {
      const body = srcOf(r);
      cache.set(r, body === undefined ? undefined : r === rel ? fn(body) : body);
    }
    return (rel2) => cache.get(rel2);
  };
  /** 造一个「某个文件根本读不到」的 srcOf（P5 负控共用：模拟权威表整块丢失）。 */
  const without = (rel) => {
    const cache = new Map();
    for (const r of allRels()) if (r !== rel) cache.set(r, srcOf(r));
    return (rel2) => cache.get(rel2);
  };
  const cases = [
    [
      "P1 面板本地兜底与种子分叉 ⇒ 点名是哪条变量",
      checkAll(patchText(panelSrc, 'b("val_pe_low", 15,', 'b("val_pe_low", 12,'), srcOf, seedSrc).some(
        (p) => p.startsWith("P1 val_pe_low") && p.includes("面板本地兜底 12") && p.includes("种子声明 15")
      ),
    ],
    [
      "P1 种子默认被改、落点没跟着改 ⇒ 红",
      checkAll(panelSrc, srcOf, patchText(seedSrc, 'name: "val_pe_high".into(),', 'name: "val_pe_high_typo".into(),')).some(
        (p) => p.includes("val_pe_high") && (p.startsWith("P4") || p.includes("P1 val_pe_high"))
      ),
    ],
    [
      "P1 落点回落值被改 ⇒ 同样点名（两侧任一处改数即红）",
      checkAll(panelSrc, withPatch(SCORING_REL, (s) => patchText(s, "rsi_oversold: 30.0", "rsi_oversold: 35.0")), seedSrc).some(
        (p) => p.startsWith("P1 signal_rsi_oversold") && p.includes("落点回落 35")
      ),
    ],
    [
      "P2 落点表里的键被改名 ⇒ 点名该条仍是空接线",
      checkAll(
        panelSrc,
        withPatch(POS_REL, (s) => patchText(s, '"pos_max_total",', '"pos_max_total_typo",')),
        seedSrc
      ).some((p) => p.startsWith("P2 pos_max_total 的键没出现在落点")),
    ],
    [
      "P2 落点构造函数没被生产路径调用 ⇒ 函数写了等于没接",
      checkAll(
        panelSrc,
        withPatch(FORMULA_REL, (s) => patchText(s, "PositionLimits::panel_effective()", "PositionLimits::default()")),
        seedSrc
      ).some((p) => p.includes("portfolio_formula.rs") && p.includes("只有 0 处")),
    ],
    [
      "P2 监控装配退回 `start()`（正门又零调用者）⇒ 红",
      checkAll(
        panelSrc,
        withPatch(SERVICES_REL, (s) => patchText(s, "monitor_arc.start_with_config(poll, cooldown).await;", "monitor_arc.start().await;")),
        seedSrc
      ).some((p) => p.startsWith("P2 ") && p.includes("start_with_config") && p.includes("services.rs")),
    ],
    [
      "P3 回放链重新抄一份 PE 20/40 ⇒ 红",
      checkAll(panelSrc, withPatch(REPLAY_REL, (s) => patchText(s, "if *pe < pe_bands.low {", "if *pe < 20.0 {")), seedSrc).some(
        (p) => p.startsWith("P3") && p.includes("*pe < 20.0")
      ),
    ],
    [
      "P3 新闻条数重新写死 50 ⇒ 红",
      checkAll(
        panelSrc,
        withPatch(MCP_REL, (s) => patchText(s, "client.get_news(stock_code, panel_news_limit()).await", "client.get_news(stock_code, 50).await")),
        seedSrc
      ).some((p) => p.startsWith("P3") && p.includes("get_news(stock_code, 50)")),
    ],
    [
      "P5 面板默认整块取不到 ⇒ 报「判据没电」而不是静默通过",
      checkAll(panelSrc.replaceAll('b("', 'zz("'), srcOf, seedSrc).some((p) => p.startsWith("P5 判据没电：面板")),
    ],
    [
      // 这一条不是「改数」也不是「摘消费点」，而是**判据自己没电**：权威表整块抽不出来时，
      // P1/P4 全部无从比对 ⇒ 必须红。缺它的话，「把 seed 那份面改名/挪走」会让门永久绿。
      "P5 声明面（seed）整块取不到 ⇒ 红而不是绿",
      checkAll(panelSrc, srcOf, undefined).some((p) => p.startsWith("P5 判据没电：seed_variables.rs")),
    ],
    [
      "P5 落点文件读不到（权威表文件缺失/搬迁）⇒ 逐条报没电，不静默通过",
      (() => {
        const problems = checkAll(panelSrc, without(MONITOR_REL), seedSrc);
        return problems.filter((p) => p.startsWith("P5 判据没电") && p.includes("monitor.rs")).length >= 1;
      })(),
    ],
    [
      // 判据自身的电门：键名只出现在 `#[cfg(test)]` 里时**不算数**（否则 A 批那些「行为锁」测试
      // 会替被删掉的生产表作证）。这条查的是**剥离器本身**，不是某个文件的当前形态：
      // 同一份源码，剥完必须「测试里的同名串消失、生产表里那一处还在」——两个方向都要成立，
      // 只查前者会退化成「剥过头把生产代码也剥了」也算通过。
      "P2 只认生产代码：剥离器必须剥掉测试里的键名、同时留住生产表那一处",
      (() => {
        const raw = read(POS_REL);
        const stripped = stripTestModules(stripComments(raw));
        const key = '"pos_max_total"';
        // 生产那一处仍在（`POSITION_LIMIT_VARS` 表里）……
        return (
          hasText(stripped, key) &&
          // ……而测试模块里那些同名串不再算数 ⇒ 剥离前后计数必须真的变小
          count(stripped, key) < count(raw, key)
        );
      })(),
    ],
    [
      "P4 落地清单被悄悄删成 8 条 ⇒ 红",
      checkAll(panelSrc, srcOf, seedSrc, LANDED.slice(1)).some((p) => p.startsWith("P4 落地清单是 8 条")),
    ],
  ];
  let pass = 0;
  for (const [label, ok] of cases) {
    console.log(`${ok ? "✓" : "✗"} ${label}`);
    if (ok) pass += 1;
  }
  console.log(`\n负控 ${pass}/${cases.length} 通过`);
  return pass === cases.length ? 0 : 1;
}

const args = process.argv.slice(2);
if (args.includes("--selftest")) {
  process.exitCode = selftest();
} else {
  const srcOf = sourceCache(allRels());
  if (args.includes("--dump")) {
    dump(srcOf);
  } else {
    process.exitCode = report(checkAll(read(PANEL_REL), srcOf, read(SEED_REL)), srcOf);
  }
}
