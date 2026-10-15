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
// ── 判据（六条）──
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
//
// ── 本门**看不见**的三件事（读数别过度解读）──
//   ① R1/R2 只管**告警**这一条事件。`portfolio-signal` 在 monitor.rs 里仍是字面量，
//      它没有前端监听点，归 PLAN 阶段 G 的门；这里不放宽成「全仓 emit 不许字面量」——
//      扫描面按域限定，不按 token 前缀全仓扫。
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

const OLD_ALERT_LITERAL = "stock-monitor-alert";
/** payload 两侧的下限：后端结构体 8 个字段，前端接口是同一份（少一键即有人抄漏）。 */
const PAYLOAD_KEYS_MIN = 8;

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

/** 从 `event_names.rs` 的 `as_str()` 实现里现取 `PriceAlertTriggered => "…"` 的字符串。 */
export function alertEventNameFromAuthority(src) {
  const m = /IpcEventName::PriceAlertTriggered\s*=>\s*"([^"]+)"/.exec(src);
  return m ? m[1] : null;
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

/** R3：两头同名 + 已进生成物。 */
export function ruleNamesMatch(authorityName, panelSrc, genNames) {
  if (!authorityName) { return { ok: false, why: "event_names.rs 取不到 PriceAlertTriggered 的字符串（判据没电）" }; }
  if (!genNames) { return { ok: false, why: "generated/events.ts 取不到 IPC_EVENT_NAMES（判据没电）" }; }
  const listens = listenedNames(panelSrc);
  if (listens.length === 0) { return { ok: false, why: `${PANEL_TSX} 里一条 listen 都抽不到（判据没电）` }; }
  if (!listens.includes(authorityName)) {
    return { ok: false, why: `面板监听的是 ${JSON.stringify(listens)}，与权威名 "${authorityName}" 不同名` };
  }
  if (!genNames.has(authorityName)) {
    return { ok: false, why: `权威名 "${authorityName}" 不在生成物 events.ts 里（生成物腐烂 ⇒ 前端类型面收不到该事件）` };
  }
  return { ok: true, authorityName };
}

/** R4：payload 两侧键集合逐项相等。 */
export function rulePayloadKeysMatch(rustFields, tsKeys) {
  if (!rustFields || rustFields.length === 0) { return { ok: false, why: "取不到 struct MonitorAlert 的字段（判据没电）" }; }
  if (!tsKeys || tsKeys.length === 0) { return { ok: false, why: "取不到 interface MonitorAlertPayload 的键（判据没电）" }; }
  if (rustFields.length < PAYLOAD_KEYS_MIN) {
    return { ok: false, why: `后端字段仅 ${rustFields.length} 个，低于下限 ${PAYLOAD_KEYS_MIN}（抽取器或结构体形态超预期）` };
  }
  if (tsKeys.length < PAYLOAD_KEYS_MIN) {
    return { ok: false, why: `前端键仅 ${tsKeys.length} 个，低于下限 ${PAYLOAD_KEYS_MIN}` };
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

// ── 跑真源文 ──

function collect() {
  const entries = scanEntries(RS_SCAN_DIRS.flatMap(walkRels));
  if (entries.length === 0) {
    return { r1: { ok: false, why: "Rust 扫描面为空（判据没电）" } };
  }
  const authorityName = alertEventNameFromAuthority(prod(EVENT_NAMES_RS));
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
  };
}

const RULES = [
  ["R1 旧字面量退役", "r1"],
  ["R2 发射点用权威枚举", "r2"],
  ["R3 两头同名且已进生成物", "r3"],
  ["R4 payload 键集合逐项相等", "r4"],
  ["R5 发射器实现必有构造点", "r5"],
  ["R6 触发状态有写有读且不用 bool 比", "r6"],
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
