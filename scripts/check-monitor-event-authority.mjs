#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
//
// 门 A · 股票告警的事件名与 payload 单一权威对账
//
// ── 本门要钉住的历史缺陷（2026-10-14 评估）──
//   发射点 `src-tauri/crates/analysis-engine/src/monitor.rs` 发的是字面量 `"stock-monitor-alert"`，
//   前端 `PriceAlertPanel` 听的是 `price-alert-triggered`，而两头之间唯一那份完整实现
//   （`src-tauri/src/init/monitor_emitter.rs`）全仓零构造点 —— 现役桥只做 `app.emit`。
//   结果：告警真的算出来了，但没人收；`price_alerts.is_triggered` 无人回写；
//   `cargo check` / `cargo test` / `tsc` 全绿。这类缺陷的形态是「跨层契约靠两边各抄一遍」，
//   所以本门不看运行期，只看两侧源文能否逐项对上。
//
// ── 判据（十七条：R1–R6 价格告警链，R7–R10 条件单链，R11 告警写库唯一入口，R12/R13 跨语言边界的键集合逐项相等，
//    R14–R16 组合级信号链，R17 轮询间隔的生效通路）──
//   R1 退役证明：旧字面量在**剥注释后的**生产 `.rs` 源文里必须零出现。
//   R2 权威在用：告警发射点必须以 `IpcEventName::PriceAlertTriggered.as_str()` 的形态发起（≥1 处）。
//   R3 两头同名：面板 `listen("…")` 的实参必须等于权威表里 `PriceAlertTriggered` 现场推导出的
//      字符串（不手抄），且该串必须已进生成物 `src/types/generated/events.ts`。
//   R4 载荷对齐：后端 `struct MonitorAlert` 的字段（snake→camel）与前端
//      `interface MonitorAlertPayload` 的键集合逐项相等，两侧各断言条数下限。
//   R5 装配存在：每个 `impl … MonitorEventEmitter for X` 的 X 都必须有构造点 ——
//      「有实现没人构造」正是本轮缺陷的另一半。构造点认两种合法写法
//      （`X::new(` 与处在实参/赋值位的 `X {`），但不裸按 `X {` 数
//      （`fn f() -> X {` 的尾巴会误命中，前一轮把 5 行虚报成 20 处就是这么来的）。
//   R6 触发闭环：`is_triggered` 的写侧（`Set(1)`）与两类读侧谓词（`eq(0)` 恢复段 / `eq(1)` 复盘段）
//      都必须在**剥注释剥 test 后的**生产源文里存在；另禁 `IsTriggered.eq(true)` —— 该列是
//      integer，PG 实测报「操作符不存在: integer = boolean」，而调用方把错误吞成「今天没有告警」。
//   R17 轮询间隔生效通路（阶段 D-2）：`start()` 的 `loop {` 之后必须仍能读到 `poll_interval_secs`
//      —— 只在循环前读一次时一切都编译、setter 也有调用者，唯独面板改了没反应。
//
// ── 本门**看不见**的三件事（读数别过度解读）──
//   ① 扫描面按域限定：只查告警 / 条件单 / 组合级信号三条链的生产 `.rs`，不按 token 前缀全仓扫
//      （`portfolio-signal` 的链已由 R14–R16 覆盖，2026-10-15 随 G-6 补上）。
//   ② R3 断的是「名字对得上」，断不了「运行期真收到」。真机核对（建一条 stop_loss 告警等它触发）
//      归阶段验收，不是门能给的保证。
//   ③ R4 断键名集合，不断**值类型**（f64 vs number、i32 vs boolean 的对账归 `npm run typecheck`
//      与 PLAN 阶段 H 的类型订正）。
//
// ── 用法 ──
//   node scripts/check-monitor-event-authority.mjs             # 门禁（0/1）
//   node scripts/check-monitor-event-authority.mjs --selftest  # 正控 + 负控，每条都要真能红/真能绿
//   node scripts/check-monitor-event-authority.mjs --dump      # 逐条打印现取读数
//
// 退出码：0 通过 ｜ 1 违规（含「判据没电」——任一侧一条都抽不到即判红，不当成全绿）

import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// 复用落地门的剥离器，不再抄第三份（那两条判据的存在理由见 check-panel-var-landing.mjs 头）。
import { stripComments, stripTestModules } from "./check-panel-var-landing.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const read = (rel) => readFileSync(join(ROOT, rel), "utf8");

/** 生产源文口径：`.rs` 先剥注释再剥 `#[cfg(test)]` 块；TS/TSX 原样（与落地门同口径）。 */
const prod = (rel) => (rel.endsWith(".rs") ? stripTestModules(stripComments(read(rel))) : read(rel));

const MONITOR_RS = "src-tauri/crates/analysis-engine/src/monitor.rs";
const EVENT_NAMES_RS = "src-tauri/crates/harness/src/event_names.rs";
const PANEL_TSX = "src/components/stock-analysis/PriceAlertPanel.tsx";
const TYPES_TS = "src/types/stock-analysis.ts";
const EVENTS_TS = "src/types/generated/events.ts";
const STOCK_ANALYSIS_RS = "src-tauri/src/commands/stock_analysis.rs";
// 阶段 G 的第三条链：组合级信号（跨股聚合器 → `RealtimeMonitor` → 组合看板）
const AGGREGATOR_RS = "src-tauri/crates/analysis-engine/src/cross_stock_aggregator.rs";
const PORTFOLIO_PANEL_TSX = "src/components/stock-analysis/PortfolioDashboard.tsx";
const SIGNAL_KEYS_MIN = 7;
const DASHBOARD_RS = "src-tauri/crates/analysis-engine/src/portfolio_monitor.rs";
// 组合 dashboard 的 TS 侧权威住在 store 文件里（`PortfolioDashboard` 接口），不在 src/types 下
const STORE_TS = "src/stores/feature/stockAnalysisStore.ts";
/** `MonitorStatusDto` 九个键 —— 少一键就是「后端发了、前端读旧键」的半改态。 */
const MONITOR_STATUS_KEYS_MIN = 9;
/** 组合 dashboard 两侧现各 19 键（含 G-1 新加的 `unpricedPositions`），下限留 18 容一次删改。 */
const DASHBOARD_KEYS_MIN = 18;

const OLD_ALERT_LITERAL = "stock-monitor-alert";
/** payload 两侧的下限：后端结构体 8 个字段，前端接口是同一份（少一键即有人抄漏）。 */
const PAYLOAD_KEYS_MIN = 8;

// 阶段 F 的第二条链：条件单触发（`conditional_orders` 表 → 行情回调 → 同一个发射器 trait）
const CONDITION_ENGINE_RS = "src-tauri/crates/analysis-engine/src/conditional_order.rs";
const CONDITION_PAYLOAD_KEYS_MIN = 9;

/**
 * R1/R5 的扫描面：`src-tauri` 下的全部生产 `.rs`。
 * 副本目录一律排除 —— `.worktrees/` `output/` `target/` 里的整棵树副本会把命中面撑大、
 * 把真正的「零命中」结论撑成假绿。
 */
const RS_SCAN_DIRS = ["src-tauri/src", "src-tauri/crates"];
const SKIP_DIR_NAMES = new Set(["target", "node_modules", ".worktrees", "output", ".git", "dist"]);

function walkRels(dir) {
  const out = [];
  for (const ent of readdirSync(join(ROOT, dir), { withFileTypes: true })) {
    if (ent.isDirectory()) {
      if (SKIP_DIR_NAMES.has(ent.name)) { continue; }
      out.push(...walkRels(`${dir}/${ent.name}`));
    } else if (ent.name.endsWith(".rs")) {
      out.push(`${dir}/${ent.name}`);
    }
  }
  return out;
}

/** 扫描面清单：生产源文以「剥过的文本 + 所属文件」成对流动，报红才点得出文件名。 */
function scanEntries(rels, strip = prod) {
  return rels.map((rel) => ({ rel, text: strip(rel) }));
}

// ── 抽取器（取不到一律返回 null / 空集 ⇒「判据没电」由调用方判红）──

/** 从权威枚举现取 `IpcEventName::<variant> => "…"` 的字符串（两条链共用）。 */
export function eventNameFromAuthority(src, variant) {
  const m = new RegExp(`IpcEventName::${variant}\\s*=>\\s*"([^"]+)"`).exec(src);
  return m ? m[1] : null;
}

/** 从 `event_names.rs` 的 `as_str()` 实现里现取 `PriceAlertTriggered => "…"` 的字符串。 */
export function alertEventNameFromAuthority(src) {
  return eventNameFromAuthority(src, "PriceAlertTriggered");
}

/** 面板里所有 `listen("名"` / `listen<T>("名"` 的实参名。 */
export function listenedNames(src) {
  const out = [];
  const re = /listen(?:<[^>]*>)?\(\s*"([^"]+)"/g;
  let m;
  while ((m = re.exec(src)) !== null) { out.push(m[1]); }
  return out;
}

/** 生成物 `IPC_EVENT_NAMES` 数组里的字符串集合。 */
export function generatedEventNames(src) {
  const body = /IPC_EVENT_NAMES\s*=\s*\[([\s\S]*?)\]/.exec(src);
  if (!body) { return null; }
  return new Set([...body[1].matchAll(/"([^"]+)"/g)].map((x) => x[1]));
}

/** Rust `pub struct <name> { … }` 的字段名（snake→camel）。 */
export function rustStructFields(src, structName) {
  const at = src.search(new RegExp(`pub struct ${structName}\\b[^{]*\\{`));
  if (at === -1) { return null; }
  const start = src.indexOf("{", at);
  let depth = 0;
  let i = start;
  for (; i < src.length; i += 1) {
    if (src[i] === "{") depth += 1;
    else if (src[i] === "}" && (depth -= 1) === 0) break;
  }
  const fields = [];
  for (const line of src.slice(start + 1, i).split("\n")) {
    const m = /^\s*pub\s+([a-z0-9_]+)\s*:/.exec(line);
    if (m) { fields.push(m[1].replace(/_([a-z0-9])/g, (_s, c) => c.toUpperCase())); }
  }
  return fields;
}

/** TS `export interface <name> { … }` 的键名。 */
export function tsInterfaceKeys(src, name) {
  const at = src.search(new RegExp(`export interface ${name}\\b[^{]*\\{`));
  if (at === -1) { return null; }
  const start = src.indexOf("{", at);
  let depth = 0;
  let i = start;
  for (; i < src.length; i += 1) {
    if (src[i] === "{") depth += 1;
    else if (src[i] === "}" && (depth -= 1) === 0) break;
  }
  const keys = [];
  for (const line of src.slice(start + 1, i).split("\n")) {
    const m = /^\s*(?:readonly\s+)?([A-Za-z0-9_]+)\??\s*:/.exec(line);
    if (m) { keys.push(m[1]); }
  }
  return keys;
}

/** `impl … MonitorEventEmitter for X` ⇒ [{ name, rel }]。 */
export function emitterImpls(entries) {
  const out = [];
  const re = /impl\s+[A-Za-z_0-9:<>]*\s*MonitorEventEmitter\s+for\s+([A-Za-z_][A-Za-z0-9_]*)/g;
  for (const { rel, text } of entries) {
    let m;
    re.lastIndex = 0;
    while ((m = re.exec(text)) !== null) { out.push({ name: m[1], rel }); }
  }
  return out;
}

/**
 * 数构造点，两种合法形态都算：`X::new(` 与**处在实参/赋值位的** struct literal `X {`。
 *
 * 为什么两种都要：本仓既有 `TauriMonitorEmitter::new(..)`，也有过 `AppMonitorEmitter { app }`
 * 这种写法，只认一种就是「锁住唯一合法的整行写法」—— 换一种等价写法会被误判成
 * 「有实现没人装配」，门随即变成常红的哑门。
 * 为什么不能裸按 `X {` 数：`fn f() -> X {` / `impl Foo for X {` / `pub struct X {` 的尾巴
 * 全都命中（前一轮据此把 5 行虚报成 20 处）。故要求 `X {` 前面是 `new(`、`::new(`、`=`、`(`、`,` 或 `return`。
 */
export function constructionCount(name, entries) {
  const literalRe = new RegExp(`(?:new\\(|=\\s*|\\(\\s*|,\\s*|return\\s+)${name}\\s*\\{`, "g");
  let n = 0;
  for (const { text } of entries) {
    n += text.split(`${name}::new(`).length - 1;
    n += (text.match(literalRe) || []).length;
  }
  return n;
}

// ── 判据（每条都是纯函数，负控才能拆在同一处）──

/** R1：旧字面量退役。 */
export function ruleOldLiteralGone(entries) {
  const hits = entries.filter(({ text }) => text.includes(`"${OLD_ALERT_LITERAL}"`));
  return hits.length === 0
    ? { ok: true }
    : { ok: false, why: `生产源文仍有 ${hits.length} 个文件含 "${OLD_ALERT_LITERAL}" 字面量：${hits.map((h) => h.rel).join("、")}` };
}

/** R2：告警发射点用权威枚举。 */
export function ruleEmitUsesAuthority(monitorProd) {
  const n = monitorProd.split("IpcEventName::PriceAlertTriggered.as_str()").length - 1;
  return n >= 1 ? { ok: true, n } : { ok: false, why: "monitor.rs 没有一处以 IpcEventName::PriceAlertTriggered.as_str() 发起告警" };
}

/** R3 / R8 / R15：两头同名 + 已进生成物（三条链共用同一条判据，只是换个变体名与面板文件）。
 *  ⚠ `panelLabel` 只用于报错文案 —— 抽不到 listen 时必须点名是哪个文件，否则红条指错地方。 */
export function ruleNamesMatch(
  authorityName,
  panelSrc,
  genNames,
  variantLabel = "PriceAlertTriggered",
  panelLabel = PANEL_TSX
) {
  if (!authorityName) { return { ok: false, why: `event_names.rs 取不到 ${variantLabel} 的字符串（判据没电）` }; }
  if (!genNames) { return { ok: false, why: "generated/events.ts 取不到 IPC_EVENT_NAMES（判据没电）" }; }
  const listens = listenedNames(panelSrc);
  if (listens.length === 0) { return { ok: false, why: `${panelLabel} 里一条 listen 都抽不到（判据没电）` }; }
  if (!listens.includes(authorityName)) {
    return { ok: false, why: `${panelLabel} 监听的是 ${JSON.stringify(listens)}，与权威名 "${authorityName}" 不同名` };
  }
  if (!genNames.has(authorityName)) {
    return { ok: false, why: `权威名 "${authorityName}" 不在生成物 events.ts 里（生成物腐烂 ⇒ 前端类型面收不到该事件）` };
  }
  return { ok: true, authorityName };
}

/** R4 / R9：payload 两侧键集合逐项相等（下限按链给，避免「抽到空集也算绿」）。 */
export function rulePayloadKeysMatch(rustFields, tsKeys, minKeys = PAYLOAD_KEYS_MIN, label = "MonitorAlert") {
  if (!rustFields || rustFields.length === 0) { return { ok: false, why: `取不到 struct ${label} 的字段（判据没电）` }; }
  if (!tsKeys || tsKeys.length === 0) { return { ok: false, why: `取不到 interface ${label}Payload 的键（判据没电）` }; }
  if (rustFields.length < minKeys) {
    return { ok: false, why: `后端字段仅 ${rustFields.length} 个，低于下限 ${minKeys}（抽取器或结构体形态超预期）` };
  }
  if (tsKeys.length < minKeys) {
    return { ok: false, why: `前端键仅 ${tsKeys.length} 个，低于下限 ${minKeys}` };
  }
  const rs = new Set(rustFields);
  const ts = new Set(tsKeys);
  const onlyRust = [...rs].filter((k) => !ts.has(k));
  const onlyTs = [...ts].filter((k) => !rs.has(k));
  if (onlyRust.length || onlyTs.length) {
    return { ok: false, why: `键集合不等：仅后端有 ${JSON.stringify(onlyRust)}，仅前端有 ${JSON.stringify(onlyTs)}` };
  }
  return { ok: true, n: rs.size };
}

/** R5：每个发射器实现都必须有构造点。 */
export function ruleEveryEmitterConstructed(impls, entries) {
  if (impls.length === 0) { return { ok: false, why: "扫描面抽不到任何 impl MonitorEventEmitter（判据没电）" }; }
  const dead = impls.filter(({ name }) => constructionCount(name, entries) === 0);
  if (dead.length) {
    return { ok: false, why: `实现存在但零构造点：${dead.map((d) => `${d.name}@${d.rel}`).join("、")} ⇒ 运行期静默失效` };
  }
  return { ok: true, emitters: impls.map((i) => i.name).join(", ") };
}

/**
 * R6：触发状态既有人写也有人读，且 integer 列不许用 bool 字面量比。
 *
 * 三条各钉一段本轮缺陷：
 * ① 写侧 `is_triggered = Set(1)` 必须存在于**生产**源文（此前唯一写侧在零构造点的文件里，
 *    剥掉 test 段后等于没有）；
 * ② 读侧两类谓词都要在 —— `eq(0)`（启动恢复只注入未触发）与 `eq(1)`（复盘取当日已触发），
 *    少任一类就意味着有一半链路在静默读空集；
 * ③ `IsTriggered.eq(true)` 一律禁止：该列在库里是 integer，PG 直接报
 *    「操作符不存在: integer = boolean」（2026-10-15 现网实测），而调用方用 `if let Ok` 把错误
 *    吞成「今天没有告警」。
 */
export function ruleTriggerStateRoundTrip(entries) {
  const boolCmp = entries.filter(({ text }) => /IsTriggered\.eq\(\s*(?:true|false)\s*\)/.test(text));
  if (boolCmp.length) {
    return {
      ok: false,
      why: `integer 列用了 bool 字面量比较（PG 报「操作符不存在: integer = boolean」）：${boolCmp.map((e) => e.rel).join("、")}`,
    };
  }
  // 字面量允许带类型后缀（本仓 `1_i32` 与 `1i32` 两种拼法都在用），判数据不判写法。
  // 后缀必须以 `_` 或字母开头 ⇒ `eq(12)` / `eq(1.5)` 不会被误认成 `1`。
  const intForm = (digit) => `${digit}(?:_[A-Za-z0-9]*|[A-Za-z][A-Za-z0-9]*)?`;
  const oneForm = new RegExp(`\\bis_triggered\\s*=\\s*Set\\(\\s*${intForm(1)}\\s*\\)`);
  const readForm = (digit) => new RegExp(`IsTriggered\\.eq\\(\\s*${intForm(digit)}\\s*\\)`);
  const writeSide = entries.filter(({ text }) => oneForm.test(text));
  if (!writeSide.length) {
    return { ok: false, why: "生产源文里没有任何一处把 is_triggered 写成 1 ⇒ 触发状态永不落库，「已触发」Tab 恒空" };
  }
  const readsZero = entries.filter(({ text }) => readForm(0).test(text));
  const readsOne = entries.filter(({ text }) => readForm(1).test(text));
  if (!readsZero.length || !readsOne.length) {
    return {
      ok: false,
      why: `读侧谓词缺一类：eq(0) ${readsZero.length} 处、eq(1) ${readsOne.length} 处（恢复段与复盘段各自都必须读）`,
    };
  }
  return { ok: true, n: `${writeSide.length} 写 / ${readsZero.length} 读0 / ${readsOne.length} 读1` };
}

/**
 * R7：条件单发射点用权威枚举，且 Rust 侧不许再出现事件名字面量。
 *
 * 与 R2 同一形态的另一条链：发射走 `IpcEventName::ConditionalOrderTriggered.as_str()`，
 * 字面量只许住在 TS 的 `listen` 形参里。两头各抄一遍正是本轮整批要消掉的根因。
 */
export function ruleConditionEmitUsesAuthority(entries, variant, authorityName) {
  if (!authorityName) { return { ok: false, why: `event_names.rs 取不到 ${variant} 的字符串（判据没电）` }; }
  const enumForm = `IpcEventName::${variant}.as_str()`;
  const emits = entries.filter(({ text }) => text.includes(enumForm));
  if (emits.length === 0) {
    return { ok: false, why: `生产源文没有一处 ${enumForm} ⇒ 条件单触发不外推（或改回了字面量）` };
  }
  const literal = entries.filter(({ rel, text }) => {
    // 权威定义文件自己那一行 `IpcEventName::<variant> => "名"` 就是字面量的**出处**，
    // 不是抄第二份。按「哪个文件持有这条映射」现场推导，不写死文件名 —— 文件搬迁不该让门假红。
    if (new RegExp(`IpcEventName::${variant}\\s*=>`).test(text)) { return false; }
    return text.includes(`"${authorityName}"`);
  });
  if (literal.length) {
    return { ok: false, why: `Rust 侧仍有事件名字面量 "${authorityName}"：${literal.map((e) => e.rel).join("、")}` };
  }
  return { ok: true, n: `${emits.length} 处发射` };
}

/**
 * R10：被退役的几只手不得回潮（反豁免计数 —— 门绿 ≠ 待修面已清，这里数的是「生产者数量必须为 0」）。
 *
 * 取代 PLAN 原写的「`None/None` 构造点计数」：阶段 F 把时间窗改成「任一侧有值即生效」，
 * `None/None` 从此是合法默认（全天），拿它当计数器会把正确写法判成待修面。
 * 判的是**生产者**：定义只能有新表这一处来源。
 */
const RETIRED_PRODUCERS = [
  ["条件单定义从分析行推理文本反读重建", /parse_condition_from_reasoning/],
  ["settings 键 conditional_orders 的写侧", /Set\(\s*"conditional_orders"/],
  ["settings 键 conditional_orders 的读侧", /find_by_id\(\s*"conditional_orders"/],
  ["把已触发意图行当条件单定义读回", /TradeIntentSource\.eq\(\s*"conditional_order"/],
  ["风控巡检服务本体（时段判据的第四份副本）", /\brisk_inspection\b/],
  ["恒返空集的假命令 conditional_order_evaluate", /conditional_order_evaluate\b/],
  // G-6：组合级信号只留「emit 给前端」这一条出口。进程内那份广播从来没有消费者
  // （`subscribe_portfolio_signals` 全仓零调用，聚合器自己的 `signal_tx` 也只写不读），
  // 留着就是「有实现没人装配」的另一种形态 —— 表面可用、实际静默。
  ["组合级信号的进程内订阅口（零消费者，已随 G-6 退役）", /subscribe_portfolio_signals\b/],
];

export function ruleRetiredProducersGone(entries) {
  const hits = [];
  for (const [label, re] of RETIRED_PRODUCERS) {
    const files = entries.filter(({ text }) => re.test(text)).map((e) => e.rel);
    if (files.length) { hits.push(`${label}：${files.join("、")}`); }
  }
  return hits.length
    ? { ok: false, why: hits.join(" ｜ ") }
    : { ok: true, n: `${RETIRED_PRODUCERS.length} 类生产者均为 0` };
}

/**
 * R11：`price_alerts` 的**新行构造点**必须只有一处，且那一处同时带着去重谓词。
 *
 * 裁定 10 的形状：同股同类型的未触发行「复用并改阈值」，判据是
 * `(stock_code, alert_type, is_triggered=0)` 三条件查回旧行。此前四个写入点各自
 * `insert` 新行 ⇒ 现网 56 行只落在 16 只股票上，每跑一次分析就多一行止盈/止损；
 * 「触发即终态」（阶段 B）之后更糟 —— 同一股的 N 条重复行仍然全部活跃，
 * 每轮各算一次、各发一条。
 *
 * 只数 struct literal，不数 `: price_alerts::ActiveModel = row.into()`：后者是把
 * **已存在**的行转成可改的 ActiveModel（触发回写走这条路），不是追加新行。
 * 同样不数返回类型尾巴 `fn f() -> price_alerts::ActiveModel {`（R5 记过的同一陷阱）。
 *
 * 「构造点与去重谓词同文件」不是洁癖：两者分家后，把复用查询删掉、只留 insert
 * 这一步在别的文件里根本看不见，门也就无从参照。
 */
const ALERT_CTOR_RE = /(?<!->\s)price_alerts::ActiveModel\s*\{/g;

export function ruleSingleAlertConstructor(entries) {
  const hits = entries
    .map(({ rel, text }) => ({ rel, n: (text.match(ALERT_CTOR_RE) || []).length }))
    .filter((h) => h.n > 0);
  const total = hits.reduce((s, h) => s + h.n, 0);
  if (total === 0) {
    return { ok: false, why: "生产源文抽不到任何 price_alerts 新行构造点（判据没电：写入点整体消失或全被剥掉）" };
  }
  if (total > 1) {
    return {
      ok: false,
      why: `新行构造点 ${total} 处（${hits.map((h) => `${h.rel}×${h.n}`).join("、")}）⇒ 各写入点自行 insert，同股同类型会无界追加行（裁定 10）`,
    };
  }
  const [{ rel }] = hits;
  if (!/IsTriggered\.eq\(/.test(entries.find((e) => e.rel === rel).text)) {
    return { ok: false, why: `唯一构造点 ${rel} 内没有 IsTriggered 去重谓词 ⇒ 复用口径与写库分家，删掉复用查询不会被察觉` };
  }
  return { ok: true, n: rel };
}

/**
 * R17：轮询间隔必须**在循环体内重读**（2026-10-15 阶段 D-2 的形态锁）。
 *
 * 形态：`start()` 的 `loop {` 之后仍出现 `poll_interval_secs.read()`。只在循环前读一次时
 * 一切照旧编译、`set_poll_interval_secs` 也照样有调用者，**只有面板改了没反应**
 * （「假旋钮」的经典形态，落地门 P2 数调用处数也照样绿 —— 所以这条必须单独锁形态）。
 * 判据只锁「读点出现在 loop 之后」，不锁写法：`interval_at` 重建、`sleep` 重算、别的方式都算。
 */
function rulePollIntervalRereadInLoop(src) {
  const startIdx = src.indexOf("pub async fn start(&self)");
  if (startIdx < 0) {
    return { ok: false, why: "monitor.rs 里找不到 `pub async fn start(&self)`（锚点失效 ⇒ 判据没电）" };
  }
  const loopIdx = src.indexOf("loop {", startIdx);
  if (loopIdx < 0) {
    return { ok: false, why: "`start()` 里找不到 `loop {`（锚点失效 ⇒ 判据没电）" };
  }
  if (!/poll_interval_secs\s*\.\s*read\(\)/.test(src.slice(loopIdx))) {
    return {
      ok: false,
      why: "`start()` 的循环体内没有再读 `poll_interval_secs` ⇒ 面板改轮询间隔要重启才生效（D-2 形态）",
    };
  }
  return { ok: true, n: "loop 内重读" };
}

// ── 跑真源文 ──

function collect() {
  const entries = scanEntries(RS_SCAN_DIRS.flatMap(walkRels));
  if (entries.length === 0) {
    return { r1: { ok: false, why: "Rust 扫描面为空（判据没电）" } };
  }
  const authorityName = alertEventNameFromAuthority(prod(EVENT_NAMES_RS));
  const condAuthority = eventNameFromAuthority(prod(EVENT_NAMES_RS), "ConditionalOrderTriggered");
  const portfolioAuthority = eventNameFromAuthority(prod(EVENT_NAMES_RS), "PortfolioSignal");
  const impls = emitterImpls(entries);
  return {
    scanned: entries.length,
    r1: ruleOldLiteralGone(entries),
    r2: ruleEmitUsesAuthority(prod(MONITOR_RS)),
    r3: ruleNamesMatch(authorityName, read(PANEL_TSX), generatedEventNames(read(EVENTS_TS))),
    r4: rulePayloadKeysMatch(
      rustStructFields(prod(MONITOR_RS), "MonitorAlert"),
      tsInterfaceKeys(read(TYPES_TS), "MonitorAlertPayload"),
    ),
    r5: ruleEveryEmitterConstructed(impls, entries),
    r6: ruleTriggerStateRoundTrip(entries),
    r7: ruleConditionEmitUsesAuthority(entries, "ConditionalOrderTriggered", condAuthority),
    r8: ruleNamesMatch(
      condAuthority,
      read(PANEL_TSX),
      generatedEventNames(read(EVENTS_TS)),
      "ConditionalOrderTriggered",
    ),
    r9: rulePayloadKeysMatch(
      rustStructFields(prod(CONDITION_ENGINE_RS), "ConditionTrigger"),
      tsInterfaceKeys(read(TYPES_TS), "ConditionTriggerPayload"),
      CONDITION_PAYLOAD_KEYS_MIN,
      "ConditionTrigger",
    ),
    r10: ruleRetiredProducersGone(entries),
    r11: ruleSingleAlertConstructor(entries),
    r12: rulePayloadKeysMatch(
      rustStructFields(prod(STOCK_ANALYSIS_RS), "MonitorStatusDto"),
      tsInterfaceKeys(read(TYPES_TS), "MonitorStatus"),
      MONITOR_STATUS_KEYS_MIN,
      "MonitorStatusDto",
    ),
    r13: rulePayloadKeysMatch(
      rustStructFields(prod(DASHBOARD_RS), "PortfolioDashboard"),
      tsInterfaceKeys(read(STORE_TS), "PortfolioDashboard"),
      DASHBOARD_KEYS_MIN,
      "PortfolioDashboard",
    ),
    // ── 第三条链：组合级信号（G-6）──
    r14: ruleConditionEmitUsesAuthority(entries, "PortfolioSignal", portfolioAuthority),
    r15: ruleNamesMatch(
      portfolioAuthority,
      read(PORTFOLIO_PANEL_TSX),
      generatedEventNames(read(EVENTS_TS)),
      "PortfolioSignal",
      PORTFOLIO_PANEL_TSX,
    ),
    r16: rulePayloadKeysMatch(
      rustStructFields(prod(AGGREGATOR_RS), "PortfolioSignal"),
      tsInterfaceKeys(read(TYPES_TS), "PortfolioSignalPayload"),
      SIGNAL_KEYS_MIN,
      "PortfolioSignal",
    ),
    // ── 阶段 D-2：轮询间隔的生效通路（形态锁）──
    r17: rulePollIntervalRereadInLoop(prod(MONITOR_RS)),
  };
}

const RULES = [
  ["R1 旧字面量退役", "r1"],
  ["R2 发射点用权威枚举", "r2"],
  ["R3 两头同名且已进生成物", "r3"],
  ["R4 payload 键集合逐项相等", "r4"],
  ["R5 发射器实现必有构造点", "r5"],
  ["R6 触发状态有写有读且不用 bool 比", "r6"],
  ["R7 条件单发射点用权威枚举且无字面量", "r7"],
  ["R8 条件单两头同名且已进生成物", "r8"],
  ["R9 条件单 payload 键集合逐项相等", "r9"],
  ["R10 退役生产者计数为 0", "r10"],
  ["R11 price_alerts 新行构造点唯一且与去重谓词同文件", "r11"],
  ["R12 monitor_status 两侧键集合逐项相等", "r12"],
  ["R13 组合 dashboard 两侧键集合逐项相等", "r13"],
  ["R14 组合级信号发射点用权威枚举且无字面量", "r14"],
  ["R15 组合级信号两头同名且已进生成物", "r15"],
  ["R16 组合级信号 payload 键集合逐项相等", "r16"],
  ["R17 轮询间隔在循环体内重读（D-2 形态锁）", "r17"],
];

function report(state) {
  console.log(`扫描面：${state.scanned} 个生产 .rs（副本目录已排除）`);
  let bad = 0;
  for (const [label, key] of RULES) {
    const res = state[key];
    if (!res) {
      bad += 1;
      console.log(`✗ ${label} —— 该条一条都没跑（判据没电）`);
      continue;
    }
    if (res.ok) {
      const extra = res.n !== undefined ? `（${res.n}）` : res.authorityName ? `（${res.authorityName}）` : res.emitters ? `（${res.emitters}）` : "";
      console.log(`✓ ${label}${extra}`);
    } else {
      bad += 1;
      console.log(`✗ ${label} —— ${res.why}`);
    }
  }
  console.log(bad ? `\n❌ ${bad} 条违规` : "\n✅ 告警事件名与 payload 单一权威对账通过");
  return bad ? 1 : 0;
}

// ── 自证：正控必须绿、负控必须红，且每条只红在自己那一格 ──

const OK_MONITOR = [
  "impl RealtimeMonitor {",
  "  async fn check_alerts(&self) {",
  "    e.emit(",
  "      IpcEventName::PriceAlertTriggered.as_str(),",
  "      serde_json::to_value(alert).unwrap(),",
  "    );",
  "  }",
  "}",
  "pub struct MonitorAlert {",
  "  pub stock_code: String,",
  "  pub stock_name: String,",
  "  pub alert_type: String,",
  "  pub alert_message: String,",
  "  pub current_price: f64,",
  "  pub change_pct: f64,",
  "  pub timestamp: String,",
  "  pub suggested_action: Option<String>,",
  "}",
  "impl MonitorEventEmitter for TauriMonitorEmitter {}",
].join("\n");

const OK_CONSTRUCTED = "fn f() { TauriMonitorEmitter::new(a, b, c); }";
const OK_PANEL_TSX = [
  "listen<MonitorAlertPayload>(\"price-alert-triggered\", (event) => {});",
  "export interface MonitorAlertPayload {",
  "  stockCode: string;",
  "  stockName: string;",
  "  alertType: string;",
  "  alertMessage: string;",
  "  currentPrice: number;",
  "  changePct: number;",
  "  timestamp: string;",
  "  suggestedAction: string | null;",
  "}",
].join("\n");
const OK_EVENTS_TS = 'export const IPC_EVENT_NAMES = ["price-alert-triggered", "agent-done"];';

// 阶段 F 第二链的齐备形态夹具（条件单触发）
const OK_COND_AUTHORITY =
  'impl IpcEventName { fn as_str(&self) { match self { IpcEventName::ConditionalOrderTriggered => "conditional-order-triggered" } } }';
const OK_COND_EMIT =
  "fn f(app: &AppHandle) { app.emit(IpcEventName::ConditionalOrderTriggered.as_str(), payload); }";
const OK_COND_STRUCT = [
  "pub struct ConditionTrigger {",
  "  pub order_id: String,",
  "  pub stock_code: String,",
  "  pub stock_name: String,",
  "  pub condition: String,",
  "  pub action: String,",
  "  pub price: f64,",
  "  pub change_pct: f64,",
  "  pub triggered_at_ms: i64,",
  "  pub reason: String,",
  "}",
].join("\n");
const OK_COND_PANEL = [
  'listen<ConditionTriggerPayload>("conditional-order-triggered", (event) => {});',
  "export interface ConditionTriggerPayload {",
  "  orderId: string;",
  "  stockCode: string;",
  "  stockName: string;",
  "  condition: string;",
  "  action: string;",
  "  price: number;",
  "  changePct: number;",
  "  triggeredAtMs: number;",
  "  reason: string;",
  "}",
].join("\n");
const OK_GEN_TWO_CHAINS =
  'export const IPC_EVENT_NAMES = ["price-alert-triggered", "conditional-order-triggered"];';
const OK_AUTHORITY = 'impl IpcEventName { fn as_str(&self) { match self { IpcEventName::PriceAlertTriggered => "price-alert-triggered" } } }';

const entry = (rel, text) => ({ rel, text });

function selftest() {
  const cases = [];
  const push = (label, got, wantOk) => cases.push({ label, got, wantOk });

  // 正控：齐备形态必须全绿
  push("正控 R1 齐备", ruleOldLiteralGone([entry("a.rs", OK_MONITOR), entry("b.rs", OK_CONSTRUCTED)]), true);
  push("正控 R2 齐备", ruleEmitUsesAuthority(OK_MONITOR), true);
  push("正控 R3 齐备", ruleNamesMatch(alertEventNameFromAuthority(OK_AUTHORITY), OK_PANEL_TSX, generatedEventNames(OK_EVENTS_TS)), true);
  push("正控 R4 齐备", rulePayloadKeysMatch(rustStructFields(OK_MONITOR, "MonitorAlert"), tsInterfaceKeys(OK_PANEL_TSX, "MonitorAlertPayload")), true);
  push("正控 R5 齐备", ruleEveryEmitterConstructed(emitterImpls([entry("m.rs", OK_MONITOR)]), [entry("m.rs", OK_MONITOR), entry("s.rs", OK_CONSTRUCTED)]), true);

  // 负控 R1：旧名回到发射点 ⇒ 红；旧名只在注释里 ⇒ 不得红（剥离器本身也是判据的一部分）
  push("负控 旧名回到发射点", ruleOldLiteralGone([entry("m.rs", OK_MONITOR.replace("IpcEventName::PriceAlertTriggered.as_str()", `"${OLD_ALERT_LITERAL}"`))]), false);
  push("负控 旧名只在注释 ⇒ 不得红", ruleOldLiteralGone([entry("m.rs", stripComments(`// 旧名 "${OLD_ALERT_LITERAL}" 已退役\n${OK_MONITOR}`))]), true);
  push("负控 旧名只在注释且不剥 ⇒ 该红（证明剥离器有电）", ruleOldLiteralGone([entry("m.rs", `// 旧名 "${OLD_ALERT_LITERAL}"\n${OK_MONITOR}`)]), false);

  // 负控 R2：发射点改回字面量 ⇒ 红
  push("负控 发射点不用权威", ruleEmitUsesAuthority(OK_MONITOR.replace("IpcEventName::PriceAlertTriggered.as_str()", '"price-alert-triggered"')), false);

  // 负控 R3：面板改名 / 生成物未重生成 ⇒ 各红
  push("负控 面板改名", ruleNamesMatch(alertEventNameFromAuthority(OK_AUTHORITY), OK_PANEL_TSX.replace(/"price-alert-triggered"/, '"alert-fired"'), generatedEventNames(OK_EVENTS_TS)), false);
  push("负控 生成物未重生成", ruleNamesMatch(alertEventNameFromAuthority(OK_AUTHORITY), OK_PANEL_TSX, generatedEventNames('export const IPC_EVENT_NAMES = ["agent-done"];')), false);
  push("负控 权威表取不到成员 ⇒ 判据没电", ruleNamesMatch(null, OK_PANEL_TSX, generatedEventNames(OK_EVENTS_TS)), false);

  // 负控 R4：一侧加/删键 ⇒ 红；抽取器名写错（判据没电）⇒ 红
  const rustWithAlertId = OK_MONITOR.replace("  pub suggested_action: Option<String>,", "  pub suggested_action: Option<String>,\n  pub alert_id: String,");
  push("负控 后端多一字段（阶段 B 的形状）", rulePayloadKeysMatch(rustStructFields(rustWithAlertId, "MonitorAlert"), tsInterfaceKeys(OK_PANEL_TSX, "MonitorAlertPayload")), false);
  push("负控 前端多一键（后端已退役）", rulePayloadKeysMatch(rustStructFields(OK_MONITOR, "MonitorAlert"), tsInterfaceKeys(OK_PANEL_TSX.replace("  changePct: number;", ""), "MonitorAlertPayload")), false);
  push("负控 结构体名抄错 ⇒ 判据没电", rulePayloadKeysMatch(rustStructFields(OK_MONITOR, "MonitorAlerts"), tsInterfaceKeys(OK_PANEL_TSX, "MonitorAlertPayload")), false);
  push("两侧同步补 alertId ⇒ 绿（本门只断键名集合，不断值类型）", rulePayloadKeysMatch(rustStructFields(rustWithAlertId, "MonitorAlert"), tsInterfaceKeys(OK_PANEL_TSX.replace("  changePct: number;", "  changePct: number;\n  alertId: string;"), "MonitorAlertPayload")), true);

  // 负控 R5：有实现没人装配 ⇒ 红；构造点只在注释里（剥完即无）⇒ 红；抽不到 impl ⇒ 红
  push("负控 有实现没人装配", ruleEveryEmitterConstructed(emitterImpls([entry("m.rs", OK_MONITOR)]), [entry("m.rs", OK_MONITOR)]), false);
  push("负控 构造点只写在注释里", ruleEveryEmitterConstructed(emitterImpls([entry("m.rs", OK_MONITOR)]), [entry("m.rs", OK_MONITOR), entry("s.rs", stripComments("// TauriMonitorEmitter::new(a, b, c);\n"))]), false);
  push("负控 扫描面抽不到 impl ⇒ 判据没电", ruleEveryEmitterConstructed([], [entry("m.rs", OK_MONITOR)]), false);

  // 构造点的两种合法写法都要认（只认一种＝锁死唯一写法，合法演进一律假红）
  push(
    "正控 struct literal 构造也算装配",
    ruleEveryEmitterConstructed(
      [{ name: "AppMonitorEmitter", rel: "m.rs" }],
      [entry("s.rs", "  let e = AppMonitorEmitter {\n    app,\n  };\n")],
    ),
    true,
  );
  push(
    "负控 只有 fn f() -> X { 与 pub struct X { ⇒ 不得算构造点",
    ruleEveryEmitterConstructed(
      [{ name: "AppMonitorEmitter", rel: "m.rs" }],
      [entry("s.rs", "fn f() -> AppMonitorEmitter {\n  todo!()\n}\npub struct AppMonitorEmitter {\n  pub app: u8,\n}\n")],
    ),
    false,
  );

  // R6：触发闭环的三格各拆一条负控（写侧、一类读侧、bool 比较）
  const OK_WRITE = "fn w() { active.is_triggered = Set(1); }";
  const OK_READ0 = "fn r0() { q.filter(Column::IsTriggered.eq(0)) }";
  const OK_READ1 = "fn r1() { q.filter(Column::IsTriggered.eq(1)) }";
  push(
    "正控 R6 齐备",
    ruleTriggerStateRoundTrip([entry("w.rs", OK_WRITE), entry("r.rs", OK_READ0), entry("d.rs", OK_READ1)]),
    true,
  );
  push(
    "负控 写侧消失（回到唯一写侧在零构造点文件里的那一年）",
    ruleTriggerStateRoundTrip([entry("r.rs", OK_READ0), entry("d.rs", OK_READ1)]),
    false,
  );
  push(
    "负控 复盘侧不再读 eq(1)",
    ruleTriggerStateRoundTrip([entry("w.rs", OK_WRITE), entry("r.rs", OK_READ0)]),
    false,
  );
  push(
    "负控 integer 列改用 bool 字面量比较",
    ruleTriggerStateRoundTrip([
      entry("w.rs", OK_WRITE),
      entry("r.rs", OK_READ0),
      entry("d.rs", "fn r1() { q.filter(Column::IsTriggered.eq(true)) }"),
    ]),
    false,
  );
  push(
    "正控 带类型后缀的写法也算（1_i32 与 1i32 本仓都在用，判数据不判写法）",
    ruleTriggerStateRoundTrip([
      entry("w.rs", "fn w() { active.is_triggered = Set(1_i32); }"),
      entry("r.rs", "fn r0() { q.filter(Column::IsTriggered.eq(0_i32)) }"),
      entry("d.rs", "fn r1() { q.filter(Column::IsTriggered.eq(1i32)) }"),
    ]),
    true,
  );

  push(
    "负控 别的整数值不得顶替 1（eq(12) 不算读了已触发）",
    ruleTriggerStateRoundTrip([
      entry("w.rs", "fn w() { active.is_triggered = Set(1); }"),
      entry("r.rs", "fn r0() { q.filter(Column::IsTriggered.eq(0)) }"),
      entry("d.rs", "fn r1() { q.filter(Column::IsTriggered.eq(12)) }"),
    ]),
    false,
  );

  // ── 阶段 F 的第二条链（条件单触发）：五格各配正控 + 拆在同一守卫上的负控 ──
  const condName = eventNameFromAuthority(OK_COND_AUTHORITY, "ConditionalOrderTriggered");
  push("正控 R7 齐备", ruleConditionEmitUsesAuthority([entry("s.rs", OK_COND_EMIT)], "ConditionalOrderTriggered", condName), true);
  push("负控 R7 发射点改回字面量", ruleConditionEmitUsesAuthority([entry("s.rs", OK_COND_EMIT.replace("IpcEventName::ConditionalOrderTriggered.as_str()", '"conditional-order-triggered"'))], "ConditionalOrderTriggered", condName), false);
  push("负控 R7 权威表取不到该成员 ⇒ 判据没电", ruleConditionEmitUsesAuthority([entry("s.rs", OK_COND_EMIT)], "ConditionalOrderTriggered", null), false);
  push("负控 R7 有枚举发射但别处又抄了一遍字面量", ruleConditionEmitUsesAuthority([entry("s.rs", OK_COND_EMIT), entry("x.rs", 'fn g() { emit("conditional-order-triggered"); }')], "ConditionalOrderTriggered", condName), false);

  push("正控 权威文件自己那行映射不算抄第二份", ruleConditionEmitUsesAuthority([entry("s.rs", OK_COND_EMIT), entry("ev.rs", OK_COND_AUTHORITY)], "ConditionalOrderTriggered", condName), true);
  push("负控 权威文件之外再抄一份字面量", ruleConditionEmitUsesAuthority([entry("s.rs", OK_COND_EMIT), entry("ev.rs", OK_COND_AUTHORITY), entry("x.rs", 'fn g() { emit("conditional-order-triggered"); }')], "ConditionalOrderTriggered", condName), false);
  push("正控 R8 齐备", ruleNamesMatch(condName, OK_COND_PANEL, generatedEventNames(OK_GEN_TWO_CHAINS), "ConditionalOrderTriggered"), true);
  push("负控 R8 面板听的是另一个名", ruleNamesMatch(condName, OK_COND_PANEL.replace('"conditional-order-triggered"', '"order-fired"'), generatedEventNames(OK_GEN_TWO_CHAINS), "ConditionalOrderTriggered"), false);
  push("负控 R8 生成物还没这条（schema-gen 没跑）", ruleNamesMatch(condName, OK_COND_PANEL, generatedEventNames(OK_EVENTS_TS), "ConditionalOrderTriggered"), false);

  push("正控 R9 齐备", rulePayloadKeysMatch(rustStructFields(OK_COND_STRUCT, "ConditionTrigger"), tsInterfaceKeys(OK_COND_PANEL, "ConditionTriggerPayload"), CONDITION_PAYLOAD_KEYS_MIN, "ConditionTrigger"), true);
  push("负控 R9 前端少一键（后端加了没同步 TS）", rulePayloadKeysMatch(rustStructFields(OK_COND_STRUCT, "ConditionTrigger"), tsInterfaceKeys(OK_COND_PANEL.replace("  reason: string;", ""), "ConditionTriggerPayload"), CONDITION_PAYLOAD_KEYS_MIN, "ConditionTrigger"), false);
  push("负控 R9 后端多一字段", rulePayloadKeysMatch(rustStructFields(OK_COND_STRUCT.replace("  pub price: f64,", "  pub price: f64,\n  pub lot: i32,"), "ConditionTrigger"), tsInterfaceKeys(OK_COND_PANEL, "ConditionTriggerPayload"), CONDITION_PAYLOAD_KEYS_MIN, "ConditionTrigger"), false);

  push("正控 R10 退役面为 0", ruleRetiredProducersGone([entry("s.rs", OK_COND_EMIT), entry("m.rs", OK_MONITOR)]), true);
  push("负控 R10 settings 读侧回潮", ruleRetiredProducersGone([entry("c.rs", 'fn r(db) { settings::Entity::find_by_id("conditional_orders").one(db) }')]), false);
  push("负控 R10 反读重建回潮", ruleRetiredProducersGone([entry("b.rs", "fn parse_condition_from_reasoning(s: &str) {}")]), false);
  push("负控 R10 风控巡检本体回潮（时段判据第四份副本）", ruleRetiredProducersGone([entry("r.rs", "use axagent_analysis_engine::risk_inspection;")]), false);
  push("负控 R10 只在注释里 ⇒ 不得红（剥离器有电）", ruleRetiredProducersGone([entry("b.rs", stripComments('// 旧实现 find_by_id("conditional_orders") 已退役\nfn ok() {}\n'))]), true);
  push("负控 R10 注释不剥 ⇒ 该红", ruleRetiredProducersGone([entry("b.rs", '// 旧实现 find_by_id("conditional_orders") 已退役\nfn ok() {}\n')]), false);

  // ── R11：新行构造点唯一（裁定 10）──
  const OK_CTOR = [
    "pub async fn upsert(db, code, kind) {",
    "  let q = Entity::find().filter(Column::IsTriggered.eq(0));",
    "  let model = price_alerts::ActiveModel {",
    "    id: Set(uuid),",
    "  };",
    "}",
  ].join("\n");
  push("正控 R11 唯一构造点且同文件带去重谓词", ruleSingleAlertConstructor([entry("up.rs", OK_CTOR), entry("m.rs", OK_MONITOR)]), true);
  push("负控 R11 第二个写入点回潮", ruleSingleAlertConstructor([entry("up.rs", OK_CTOR), entry("w.rs", "fn c() { let m = price_alerts::ActiveModel { id: Set(x) }; }")]), false);
  push("负控 R11 构造点与去重谓词分家", ruleSingleAlertConstructor([entry("up.rs", OK_CTOR.replace("  let q = Entity::find().filter(Column::IsTriggered.eq(0));", ""))]), false);
  push("负控 R11 只剩 into()/返回类型尾巴 ⇒ 判据没电", ruleSingleAlertConstructor([
    entry("e.rs", "fn w(a) { let mut active: price_alerts::ActiveModel = a.into(); }"),
    entry("f.rs", "fn make() -> price_alerts::ActiveModel {\n  todo!()\n}"),
  ]), false);
  push("负控 R11 同一文件里两处 insert 也要算两处（改造前 core.rs 的形状）", ruleSingleAlertConstructor([
    entry("up.rs", OK_CTOR),
    entry("c.rs", "fn a() { let m = price_alerts::ActiveModel {\n  id: Set(1),\n}; }\nfn b() { let m = price_alerts::ActiveModel {\n  id: Set(2),\n}; }"),
  ]), false);
  push("正控 into() 与返回类型尾巴不得被算成构造点", ruleSingleAlertConstructor([
    entry("up.rs", OK_CTOR),
    entry("e.rs", "fn w(a) { let mut active: price_alerts::ActiveModel = a.into(); }"),
    entry("f.rs", "fn make() -> price_alerts::ActiveModel {\n  todo!()\n}"),
  ]), true);

  // ── R12：monitor_status 两头键集合（裁定 12 的新命令）──
  const OK_STATUS_RS = [
    "pub struct MonitorStatusDto {",
    "  pub assembled: bool,",
    "  pub session: String,",
    "  pub trading_now: bool,",
    "  pub poll_interval_secs: u64,",
    "  pub configs_active: u32,",
    "  pub round_codes_total: u32,",
    "  pub round_codes_failed: u32,",
    "  pub consecutive_all_failed_rounds: u32,",
    "  pub round_finished_at_ms: i64,",
    "}",
  ].join("\n");
  const OK_STATUS_TS = [
    "export interface MonitorStatus {",
    "  assembled: boolean;",
    "  session: string;",
    "  tradingNow: boolean;",
    "  pollIntervalSecs: number;",
    "  configsActive: number;",
    "  roundCodesTotal: number;",
    "  roundCodesFailed: number;",
    "  consecutiveAllFailedRounds: number;",
    "  roundFinishedAtMs: number;",
    "}",
  ].join("\n");
  push(
    "正控 R12 snake→camel 逐项相等",
    rulePayloadKeysMatch(
      rustStructFields(OK_STATUS_RS, "MonitorStatusDto"),
      tsInterfaceKeys(OK_STATUS_TS, "MonitorStatus"),
      MONITOR_STATUS_KEYS_MIN,
      "MonitorStatusDto",
    ),
    true,
  );
  push(
    "负控 R12 前端漏一条键（后端发了、面板读的是旧键 ⇒ 那一句永远不出）",
    rulePayloadKeysMatch(
      rustStructFields(OK_STATUS_RS, "MonitorStatusDto"),
      tsInterfaceKeys(OK_STATUS_TS.replace("  consecutiveAllFailedRounds: number;", ""), "MonitorStatus"),
      MONITOR_STATUS_KEYS_MIN,
      "MonitorStatusDto",
    ),
    false,
  );
  push(
    "负控 R12 前端写成 snake_case（消费端拿不到值，正是本轮根因家族）",
    rulePayloadKeysMatch(
      rustStructFields(OK_STATUS_RS, "MonitorStatusDto"),
      tsInterfaceKeys(OK_STATUS_TS.replace("  tradingNow: boolean;", "  trading_now: boolean;"), "MonitorStatus"),
      MONITOR_STATUS_KEYS_MIN,
      "MonitorStatusDto",
    ),
    false,
  );
  push(
    "负控 R12 结构体名抄错 ⇒ 判据没电（不许由「抽不到」推成「对齐」）",
    rulePayloadKeysMatch(
      rustStructFields(OK_STATUS_RS, "MonitorStatus"),
      tsInterfaceKeys(OK_STATUS_TS, "MonitorStatus"),
      MONITOR_STATUS_KEYS_MIN,
      "MonitorStatusDto",
    ),
    false,
  );

  // ── R13：组合 dashboard 两侧键集合（G-1 新加 unpricedPositions 就是这一格要管的）──
  const OK_DASH_RS = [
    "pub struct PortfolioDashboard {",
    "  pub is_historical: bool,",
    "  pub as_of_date: Option<String>,",
    "  pub total_market_value: f64,",
    "  pub total_pnl: f64,",
    "  pub total_pnl_pct: f64,",
    "  pub unpriced_positions: u32,",
    "  pub cash_pct: f64,",
    "  pub max_drawdown_pct: f64,",
    "  pub beta: Option<f64>,",
    "  pub sharpe_30d: Option<f64>,",
    "  pub correlation_avg: Option<f64>,",
    "  pub top_concentration_pct: f64,",
    "  pub sector_exposure: HashMap<String, f64>,",
    "  pub concentration_warning: Option<String>,",
    "  pub risk_level: String,",
    "  pub diversification_score: u32,",
    "  pub stress_test: StressTestBundle,",
    "  pub positions: Vec<PositionSummary>,",
    "  pub snapshot_at: i64,",
    "}",
  ].join("\n");
  const OK_DASH_TS = [
    "export interface PortfolioDashboard {",
    "  isHistorical: boolean;",
    "  asOfDate?: string;",
    "  totalMarketValue: number;",
    "  totalPnl: number;",
    "  totalPnlPct: number;",
    "  unpricedPositions: number;",
    "  cashPct: number;",
    "  maxDrawdownPct: number;",
    "  beta?: number;",
    "  sharpe30d?: number;",
    "  correlationAvg?: number;",
    "  topConcentrationPct: number;",
    "  sectorExposure: Record<string, number>;",
    "  concentrationWarning?: string;",
    "  riskLevel: string;",
    "  diversificationScore: number;",
    "  stressTest: PortfolioStressBundle;",
    "  positions: PortfolioPositionRow[];",
    "  snapshotAt: number;",
    "}",
  ].join("\n");
  const dashRule = (tsSrc) =>
    rulePayloadKeysMatch(
      rustStructFields(OK_DASH_RS, "PortfolioDashboard"),
      tsInterfaceKeys(tsSrc, "PortfolioDashboard"),
      DASHBOARD_KEYS_MIN,
      "PortfolioDashboard",
    );
  push("正控 R13 齐备", dashRule(OK_DASH_TS), true);
  push(
    "负控 R13 后端加了字段、前端类型没跟（G-1 当天就会撞的形状）",
    dashRule(OK_DASH_TS.replace("  unpricedPositions: number;", "")),
    false,
  );
  push("负控 R13 前端多出一个后端没有的键", dashRule(OK_DASH_TS.replace("  riskLevel: string;", "  riskLevel: string;\n  legacyRiskText: string;")), false);

  // ── R14/R15/R16：第三条链（组合级信号，G-6）──
  const OK_SIG_AUTHORITY =
    'impl IpcEventName { fn as_str(&self) { match self { IpcEventName::PortfolioSignal => "portfolio-signal" } } }';
  const OK_SIG_EMIT =
    "fn f(e: &dyn MonitorEventEmitter, s: &PortfolioSignal) { e.emit(IpcEventName::PortfolioSignal.as_str(), to_value(s)); }";
  const OK_SIG_STRUCT = [
    "pub struct PortfolioSignal {",
    "  pub direction: SignalDirection,",
    "  pub stocks: Vec<String>,",
    "  pub dominant_signal: SignalType,",
    "  pub strength: f64,",
    "  pub snapshot: Option<PortfolioSnapshot>,",
    "  pub suggested_action: String,",
    "  pub timestamp: i64,",
    "}",
  ].join("\n");
  const OK_SIG_PANEL = [
    'listen<PortfolioSignalPayload>("portfolio-signal", (event) => {});',
    "export interface PortfolioSignalPayload {",
    "  direction: string;",
    "  stocks: string[];",
    "  dominantSignal: string;",
    "  strength: number;",
    "  snapshot?: PortfolioSnapshotPayload | null;",
    "  suggestedAction: string;",
    "  timestamp: number;",
    "}",
  ].join("\n");
  const sigName = eventNameFromAuthority(OK_SIG_AUTHORITY, "PortfolioSignal");
  const OK_GEN_SIG = 'export const IPC_EVENT_NAMES = ["portfolio-signal"];';
  push("正控 R14 齐备", ruleConditionEmitUsesAuthority([entry("s.rs", OK_SIG_EMIT)], "PortfolioSignal", sigName), true);
  push("负控 R14 发射点退回字面量", ruleConditionEmitUsesAuthority([entry("s.rs", OK_SIG_EMIT.replace("IpcEventName::PortfolioSignal.as_str()", '"portfolio-signal"'))], "PortfolioSignal", sigName), false);
  push("负控 R14 权威表没这个成员 ⇒ 判据没电", ruleConditionEmitUsesAuthority([entry("s.rs", OK_SIG_EMIT)], "PortfolioSignal", null), false);
  push("正控 R15 齐备", ruleNamesMatch(sigName, OK_SIG_PANEL, generatedEventNames(OK_GEN_SIG), "PortfolioSignal", "dash.tsx"), true);
  push("负控 R15 看板没在听（回到零消费者的那一年）", ruleNamesMatch(sigName, OK_SIG_PANEL.replace(/"portfolio-signal"/, '"sig-fired"'), generatedEventNames(OK_GEN_SIG), "PortfolioSignal", "dash.tsx"), false);
  push("负控 R15 生成物没这条（schema-gen 没跑）", ruleNamesMatch(sigName, OK_SIG_PANEL, generatedEventNames('export const IPC_EVENT_NAMES = ["agent-done"];'), "PortfolioSignal", "dash.tsx"), false);
  push(
    "正控 R16 齐备",
    rulePayloadKeysMatch(
      rustStructFields(OK_SIG_STRUCT, "PortfolioSignal"),
      tsInterfaceKeys(OK_SIG_PANEL, "PortfolioSignalPayload"),
      SIGNAL_KEYS_MIN,
      "PortfolioSignal",
    ),
    true,
  );
  push(
    "负控 R16 后端加字段、TS 没跟",
    rulePayloadKeysMatch(
      rustStructFields(OK_SIG_STRUCT.replace("  pub timestamp: i64,", "  pub timestamp: i64,\n  pub window_secs: u32,"), "PortfolioSignal"),
      tsInterfaceKeys(OK_SIG_PANEL, "PortfolioSignalPayload"),
      SIGNAL_KEYS_MIN,
      "PortfolioSignal",
    ),
    false,
  );
  push(
    "负控 R10 组合级信号的进程内订阅口回潮",
    ruleRetiredProducersGone([entry("m.rs", "pub async fn subscribe_portfolio_signals(&self) {}")]),
    false,
  );

  // ── R17：轮询间隔生效通路（形态锁，D-2）──
  // 正控认两种写法：`interval_at` 重建 与 `sleep` 重算 —— 判据只锁「loop 之后还有读点」。
  const OK_POLL_REREAD = [
    "impl RealtimeMonitor {",
    "  pub async fn start(&self) {",
    "    let mut ticker = interval(Duration::from_secs(*self.poll_interval_secs.read().await));",
    "    loop {",
    "      ticker.tick().await;",
    "      let want = (*self.poll_interval_secs.read().await).max(1);",
    "    }",
    "  }",
    "}",
  ].join("\n");
  push("正控 R17 循环内重读齐备", rulePollIntervalRereadInLoop(OK_POLL_REREAD), true);
  push(
    "负控 R17 只在循环前读一次（D-2 的原形态）",
    rulePollIntervalRereadInLoop(
      OK_POLL_REREAD.replace("      let want = (*self.poll_interval_secs.read().await).max(1);\n", ""),
    ),
    false,
  );
  push(
    "正控 R17 另一种合法写法（sleep 重算，不绑 interval_at）",
    rulePollIntervalRereadInLoop([
      "impl RealtimeMonitor {",
      "  pub async fn start(&self) {",
      "    loop {",
      "      tokio::time::sleep(Duration::from_secs(*self.poll_interval_secs.read().await)).await;",
      "    }",
      "  }",
      "}",
    ].join("\n")),
    true,
  );
  push(
    "负控 R17 锚点失效（start 被改名）⇒ 判据没电也算红",
    rulePollIntervalRereadInLoop(OK_POLL_REREAD.replace("pub async fn start(&self)", "pub async fn start_renamed(&self)")),
    false,
  );

  let pass = 0;
  for (const c of cases) {
    const ok = c.got.ok === c.wantOk;
    if (ok) { pass += 1; }
    console.log(`${ok ? "✓" : "✗"} ${c.label}（期望${c.wantOk ? "绿" : "红"}｜实测${c.got.ok ? "绿" : "红"}${c.got.ok ? "" : ` — ${c.got.why}`}）`);
  }
  console.log(`\n自证 ${pass}/${cases.length} 通过`);
  return pass === cases.length ? 0 : 1;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  if (args.includes("--selftest")) {
    process.exitCode = selftest();
  } else if (args.includes("--dump")) {
    console.log(JSON.stringify(collect(), null, 2));
  } else {
    process.exitCode = report(collect());
  }
}
