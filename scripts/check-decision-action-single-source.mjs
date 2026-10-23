// 决策结论档「单一权威 + 分档互斥」对账门（PLAN-decision-action-single-source.md §2 阶段 A1）
//
// ── 为什么必须有这道门 ──
// 用户 2026-10-22 裁定：「项目全局只能存在一处权威定义，且定义内部分档不能有重叠和语义模糊」。
// 同一「操作档位」语义在本仓散成多套互不相交的清单（`harness/src/decision_action.rs` 文件头
// 自己列了五套），且**没有任何锁**：改定义端不会让任何一处报错，失效形态是
// 「前端把后端合法值折成不确定」「风控遇英文值域 fail-open 直接放行」「同义档位进两个回测策略」。
// 别名集合不同集合已实证：后端认 `watch`/`clear`/`trim`/`strong_buy`，前端 `parseActionStrict`
// 不认 ⇒ 同一条记录后端有档、界面显示「不确定」。
//
// ── 权威是谁 ──
// `src-tauri/crates/harness/src/decision_action.rs` 的 `ActionKind`（AGENTS.md 规则 12：
// 共享契约的权威定义在 axagent-harness）。其余七面一律**被对账**，不得自带清单：
// 前端 `StockAction` / `STOCK_ACTION_LABELS` / `getActionTKey`、前端 `STOCK_ACTION_CANONICAL_LABELS`
// （第二份「枚举→中文」表）、前端 `ACTION_PALETTE`（档位→强度配色）、LLM 输出 schema、
// Rhai 与提示词的产端字面量、11 份 locale 的标签、来历权威 `harness/src/analysis_kind.rs`、
// DB 侧 `CHECK` 清单（`dao/reconcile/extras.rs`）。
//
// ── 判据 ──
//   R0 源文件读不到、或任一抽取器解析不到 ⇒ 直接红（解析失效不得当作「空集=通过」）
//   R1 前端 `StockAction` 的 token 集合必须等于 Rust `as_token` 的 token 集合
//   R2 Rust `as_storage_cn` 的八档中文必须与 zh-CN 的 `stockAnalysis.action*` 逐档同字
//   R3 `getActionTKey` 的八个 i18n 键必须在 11 份 locale 全部存在且非空
//   R4 非中英语言的档位标签不得仍是英文 token（未译 = 同一档两种显示）
//   R5 `llm-output-schemas.json` 里每处 action enum 必须彼此一致，且等于权威「六档操作建议」
//   R6 后端 `ACTION_ALIASES` 的每一项都必须被前端严格解析表覆盖（别名不同集合 = 前端降级为不确定）
//   R7 产端（Rhai 决策脚本 / 专家提示词）不得输出**别名档名**，只能输出权威中文
//   R8 不得存在第二套同义 i18n 键（`stockAnalysis.conditional.action*` 是 R6 退役项的计数器）
//   R9 八档在 `getActionTKey` 里必须各有显式 case，只被 default 兜住的档 = 机器读不到它的键 ⇒ 红
//   R10 前端 `STOCK_ACTION_CANONICAL_LABELS`（第二份「枚举→中文」表）必须逐档等于权威 `as_storage_cn`
//   R11 强度配色 `ACTION_PALETTE`：八档齐、**八色互不相同**、六档 rank 恰好 0..5、
//      兜底两档 rank=null，且「利空→利好」的先后必须与权威谓词 `implies_reduce` / `implies_buy` 同向
//   R12 按档位分支（`case StockAction.X:`）与第二份 `ACTION_PALETTE` 只允许存在于权威文件里
//   R13 记录来历值域（`live` / `replay`）另有自己的权威 `harness/src/analysis_kind.rs`：
//      Rust 消费端（含测试夹具）不得写这两种字面量，前端 `provenanceOfRecord` 认的值集合必须与权威相同。
//      ⚠ 与 `useTimeAnchorStore.anchorMode` 同词不同源 —— 那是查看者的时间锚，不是记录来历，门不管它。
//   R14 DB 侧 `CHECK`（`dao/reconcile/extras.rs`）里那两条 SQL 值域清单，必须逐档等于
//      `ANALYSIS_KIND_VALUES` 与 `as_storage_cn` —— 值域在 Rust / DB / 前端共三处表达，
//      DB 这条是**第三处**，用门钉住而不是靠自觉（同 R2/R10 的手法）。
//
// ── 用法 ──
//   node scripts/check-decision-action-single-source.mjs            # 门禁模式
//   node scripts/check-decision-action-single-source.mjs --selftest # 二十一条负控（每条必须真的能红）
//   node scripts/check-decision-action-single-source.mjs --dump     # 打十四份抽取结果，排漂移用
// 退出码：0 = 通过；1 = 有违规或负控失效。
//
// ⚠ 解析纪律（沿用本仓既有门的三条教训）：
//   ① 扫源码前先**剥注释**（等长替换），否则注释里的散文会把别名当产端字面量触红；
//   ② 解析不到 = **当红**，不得当空集（空集会让门假绿）；
//   ③ 判据全部打在**纯函数**上（入参是源码字符串），负控靠改字符串自证有电，不落盘、不碰共享树。

import { readdirSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const read = (rel) => readFileSync(resolve(ROOT, rel), "utf8");

const AUTHORITY_RS = "src-tauri/crates/harness/src/decision_action.rs";
/** 记录来历（`live` / `replay`）的权威 —— 与 action 是两个值域，各自一处定义 */
const AUTHORITY_KIND_RS = "src-tauri/crates/harness/src/analysis_kind.rs";
/** DB 侧值域 `CHECK` 声明（第三处表达，靠 R14 与两个权威逐档对账） */
const EXTRAS_RS = "src-tauri/crates/dao/src/reconcile/extras.rs";
const FRONTEND_TS = "src/lib/stock-analysis-utils.ts";
const LLM_SCHEMA_JSON = "src-tauri/config/llm-output-schemas.json";
const ZH_LOCALE = "src/i18n/locales/zh-CN.json";
const LOCALES = ["ar", "de", "en-US", "es", "fr", "hi", "ja", "ko", "ru", "zh-CN", "zh-TW"];
/** 中文界面语言：这些语言的标签**应当**等于权威中文，不参与 R4 的「未译」判定 */
const CJK_LOCALES = new Set(["zh-CN", "zh-TW"]);
const RHAI_SOURCES = [
  "src-tauri/src/commands/portfolio-mgr.rhai",
  "src-tauri/src/commands/portfolio-mgr-h-ultra-short.rhai",
  "src-tauri/src/commands/portfolio-mgr-h-short.rhai",
  "src-tauri/src/commands/portfolio-mgr-h-mid.rhai",
  "src-tauri/src/commands/portfolio-mgr-h-long.rhai",
];
const PROMPT_SOURCES = [
  "src-tauri/agency_experts/stock-analysis/trader.md",
  "src-tauri/agency_experts/stock-analysis/custom/value-investor.md",
];

/** ActionKind 变体（权威顺序）；R1/R2/R5 都按这个顺序逐档对账 */
const VARIANTS = ["Buy", "Increase", "Hold", "Wait", "Reduce", "Sell", "Uncertain", "Unavailable"];
/** 六档「操作建议」（`as_cn` 有值的那六档）；LLM 与公式产端只允许输出这六档 */
const OPERATING = ["Buy", "Increase", "Hold", "Wait", "Reduce", "Sell"];

/** 等长剥注释：行注释与块注释都换成空格，保持字节位移不变（判据不得被注释触红） */
export function stripComments(src) {
  const out = src.split("");
  let i = 0;
  let mode = "code"; // code | line | block | str
  while (i < out.length) {
    const c = out[i];
    const n = out[i + 1];
    if (mode === "code") {
      if (c === "/" && n === "/") {
        mode = "line";
        out[i] = " ";
        out[i + 1] = " ";
        i += 2;
        continue;
      }
      if (c === "/" && n === "*") {
        mode = "block";
        out[i] = " ";
        out[i + 1] = " ";
        i += 2;
        continue;
      }
      if (c === '"') { mode = "str"; }
      i += 1;
      continue;
    }
    if (mode === "line") {
      if (c === "\n") { mode = "code"; } else { out[i] = " "; }
      i += 1;
      continue;
    }
    if (mode === "block") {
      if (c === "*" && n === "/") {
        out[i] = " ";
        out[i + 1] = " ";
        mode = "code";
        i += 2;
        continue;
      }
      if (c !== "\n") { out[i] = " "; }
      i += 1;
      continue;
    }
    // 字符串内：处理转义，注释符不算注释
    if (c === "\\") { i += 2; continue; }
    if (c === '"') { mode = "code"; }
    i += 1;
  }
  return out.join("");
}

/** 取 `sig` 之后第一个平衡 `{…}` 体（含首尾花括号）；取不到返回 null（调用方按红处理） */
function bodyAfter(src, sig) {
  const at = src.indexOf(sig);
  if (at < 0) { return null; }
  const open = src.indexOf("{", at + sig.length);
  if (open < 0) { return null; }
  let depth = 0;
  for (let i = open; i < src.length; i += 1) {
    if (src[i] === "{") { depth += 1; } else if (src[i] === "}") {
      depth -= 1;
      if (depth === 0) { return src.slice(open, i + 1); }
    }
  }
  return null;
}

/** Rust：`pub fn as_token` / `as_cn` / `as_storage_cn` ⇒ 变体 → 值（值为 null 表示 `None`） */
export function rustVariantMap(rsSrc, fnName) {
  const body = bodyAfter(rsSrc, `pub fn ${fnName}(`);
  if (!body) { return null; }
  const map = {};
  // 四种右值形态：`None` / `Some("买入")` / `"BUY"` / `ACTION_UNAVAILABLE`
  const re = /Self::(\w+)(?:\s*\|\s*Self::(\w+))?\s*=>\s*(None|Some\("([^"]*)"\)|"([^"]*)"|ACTION_UNAVAILABLE)/g;
  let m;
  while ((m = re.exec(body)) !== null) {
    const variants = m[2] ? [m[1], m[2]] : [m[1]];
    const val = m[3] === "None"
      ? null
      : m[4] !== undefined
        ? m[4]
        : m[5] !== undefined
          ? m[5]
          : "UNAVAILABLE";
    for (const v of variants) { map[v] = val; }
  }
  return Object.keys(map).length ? map : null;
}

/** Rust：`pub const ACTION_ALIASES: &[&str] = &[ … ]` ⇒ 字面量数组 */
export function rustStringArray(rsSrc, constName) {
  const at = rsSrc.indexOf(`pub const ${constName}`);
  if (at < 0) { return null; }
  const open = rsSrc.indexOf("[", at);
  const close = rsSrc.indexOf("];", open);
  if (open < 0 || close < 0) { return null; }
  const seg = rsSrc.slice(open, close);
  const items = [...seg.matchAll(/"([^"]*)"/g)].map((x) => x[1]);
  return items.length ? items : null;
}

/** Rust：`pub fn implies_buy(self)` 体里的 `Self::Buy | Self::Increase` ⇒ 变体名数组（去重） */
export function rustPredicateSet(rsSrc, fnName) {
  const body = bodyAfter(rsSrc, `pub fn ${fnName}(`);
  if (!body) { return null; }
  const variants = [...new Set([...body.matchAll(/Self::([A-Z]\w+)/g)].map((m) => m[1]))];
  return variants.length ? variants : null;
}

/** TS：`export const ACTION_PALETTE` ⇒ 档成员名 → `{ hex, rank }`（rank 可为 null） */
export function tsPaletteMap(tsSrc) {
  const at = tsSrc.indexOf("export const ACTION_PALETTE");
  if (at < 0) { return null; }
  // 类型注解里也带花括号（`Record<StockActionType, { readonly hex… }>`) ⇒ 不能取第一个 `{`，
  // 要取行尾 `= {` 那一对 —— 它是对象字面量的起点（首跑就是红在这里，抽取器比判据先错）。
  const eq = tsSrc.indexOf("= {", at);
  if (eq < 0) { return null; }
  // `bodyAfter(_, "")` ⇒ 从切片第一个字符处开始找 `{`（这里 slice 已以 `{` 开头，
  // 传 `"{"` 反而会把 sig 自身跳过、取到第一个**条目**的花括号）
  const body = bodyAfter(tsSrc.slice(eq + 2), "");
  if (!body) { return null; }
  const map = {};
  const re = /\[StockAction\.([A-Z_]+)\]\s*:\s*\{\s*hex:\s*"([^"]+)"\s*,\s*rank:\s*(null|-?\d+),?\s*\}/g;
  let m;
  while ((m = re.exec(body)) !== null) {
    map[m[1]] = { hex: m[2], rank: m[3] === "null" ? null : Number(m[3]) };
  }
  return Object.keys(map).length ? map : null;
}

/** TS：`export const StockAction = { BUY: "BUY", … } as const` ⇒ 键 → token */
export function tsConstMap(tsSrc, name) {
  const body = bodyAfter(tsSrc, `export const ${name} =`);
  if (!body) { return null; }
  const map = {};
  for (const m of body.matchAll(/^\s*([A-Z_]+)\s*:\s*"([^"]*)",\s*$/gm)) { map[m[1]] = m[2]; }
  return Object.keys(map).length ? map : null;
}

/** TS：`STOCK_ACTION_LABELS` ⇒ 别名字面量 → 档 token */
export function tsLabelMap(tsSrc) {
  const body = bodyAfter(tsSrc, "export const STOCK_ACTION_LABELS");
  if (!body) { return null; }
  const map = {};
  for (const m of body.matchAll(/^\s*"([^"]+)"\s*:\s*StockAction\.([A-Z_]+)\s*,\s*$/gm)) {
    map[m[1]] = m[2];
  }
  return Object.keys(map).length ? map : null;
}

/**
 * TS：`getActionTKey` ⇒ `{ map, defaultKey }`
 * map = 档 token → i18n 键（**只收显式 case**）；defaultKey = `default:` 那一条兜底键。
 *
 * 刻意不把 default 当成某档的映射：靠 default 兜住的档位，门与人都读不到「它对应哪个键」，
 * 将来 `ActionKind` 加一档而 `getActionTKey` 忘记加 case 时，症状是那一档静默显示成「不确定」。
 */
export function tsActionKeyMap(tsSrc) {
  const body = bodyAfter(tsSrc, "export function getActionTKey(");
  if (!body) { return null; }
  const map = {};
  const re = /case StockAction\.([A-Z_]+):\s*(?:\{\s*)?return\s+"([^"]+)"/g;
  let m;
  while ((m = re.exec(body)) !== null) {
    if (!map[m[1]]) { map[m[1]] = m[2]; }
  }
  const def = /default:\s*(?:\{\s*)?return\s+"([^"]+)"/.exec(body);
  return Object.keys(map).length ? { map, defaultKey: def ? def[1] : null } : null;
}

/** TS：`STOCK_ACTION_CANONICAL_LABELS` ⇒ 档 token → 中文（第二份「枚举→中文」表，R10 对账） */
export function tsCanonicalLabelMap(tsSrc) {
  const body = bodyAfter(tsSrc, "export const STOCK_ACTION_CANONICAL_LABELS");
  if (!body) { return null; }
  const map = {};
  for (const m of body.matchAll(/\[StockAction\.([A-Z_]+)\]\s*:\s*"([^"]*)"/g)) { map[m[1]] = m[2]; }
  return Object.keys(map).length ? map : null;
}

/** 把文本切成 token（保留中文词与 ASCII 串），用于「产端是否输出别名档名」的判定 */
export function tokensOf(src) {
  return new Set(src.split(/[^0-9A-Za-z一-鿿]+/).filter(Boolean));
}

/** JSON：所有 `"action": { … "enum": [ … ] }` 的 enum 数组（连出现位置行号） */
export function schemaActionEnums(jsonSrc) {
  const lines = jsonSrc.split("\n");
  const found = [];
  for (let i = 0; i < lines.length; i += 1) {
    if (!/^\s*"action"\s*:\s*\{/.test(lines[i])) { continue; }
    let j = i;
    let enumAt = -1;
    while (j < lines.length && j < i + 30) {
      if (/"enum"\s*:\s*\[/.test(lines[j])) { enumAt = j; break; }
      if (/^\s*\},?\s*$/.test(lines[j]) && j > i) { break; }
      j += 1;
    }
    if (enumAt < 0) { continue; }
    const vals = [];
    for (let k = enumAt + 1; k < lines.length; k += 1) {
      if (/^\s*\]/.test(lines[k])) { break; }
      const m = lines[k].match(/^\s*"([^"]+)"[,\s]*$/);
      if (m) { vals.push(m[1]); }
    }
    found.push({ line: enumAt + 1, values: vals });
  }
  return found;
}

/** 从 locale JSON 里取 stockAnalysis.action* 八个值（缺 → undefined） */
function localeActionValues(localeJson, keyMap) {
  const data = JSON.parse(localeJson);
  const sa = data.stockAnalysis ?? {};
  const out = {};
  for (const [token, key] of Object.entries(keyMap)) {
    const leaf = key.split(".").pop();
    out[token] = typeof sa[leaf] === "string" ? sa[leaf].trim() : undefined;
  }
  return out;
}

const sameSet = (a, b) => a.length === b.length && a.every((x) => b.includes(x));
const isAsciiToken = (v) => /^[A-Za-z_ ]+$/.test(v);

/**
 * 主判据：入参全是源码字符串（纯函数），负控靠改字符串自证有电。
 * 返回违规清单，每项 `{ rule, msg }`。
 */
export function checkAll(parts) {
  const bad = [];
  const push = (rule, msg) => bad.push({ rule, msg });

  // 声明的源文件读不到 = 判据失效，当红（门不得因为文件搬家而假绿）
  for (const m of parts.missing ?? []) { push("R0", `源文件读不到：${m}`); }

  const rs = stripComments(parts.authorityRs ?? "");
  const ts = stripComments(parts.frontendTs ?? "");

  const tokens = rustVariantMap(rs, "as_token");
  const storageCn = rustVariantMap(rs, "as_storage_cn");
  const cn = rustVariantMap(rs, "as_cn");
  const aliases = rustStringArray(rs, "ACTION_ALIASES");
  const tsTokens = tsConstMap(ts, "StockAction");
  const tsLabels = tsLabelMap(ts);
  const tsCanonical = tsCanonicalLabelMap(ts);
  const tsKeyMap = tsActionKeyMap(ts);

  // 解析失败一律当红（不当「无违规」）
  for (const [name, got] of [["as_token", tokens], ["as_storage_cn", storageCn], ["as_cn", cn],
    ["ACTION_ALIASES", aliases], ["StockAction", tsTokens], ["STOCK_ACTION_LABELS", tsLabels],
    ["STOCK_ACTION_CANONICAL_LABELS", tsCanonical],
    ["getActionTKey", tsKeyMap]]) {
    if (!got) { push("R0", `解析不到 ${name} ⇒ 判据失效，不得当通过`); }
  }
  if (bad.length) { return bad; }

  // 八档各自的 i18n 键：只认显式 case；只被 default 兜住的档 ⇒ R9 红（机器读不到它的键）
  let keyIncomplete = false;
  const keyMap = {};
  for (const v of VARIANTS) {
    const token = tokens[v];
    const explicit = tsKeyMap.map[token];
    if (!explicit) {
      keyIncomplete = true;
      push("R9", `${token} 在 getActionTKey 里没有显式 case，只被 default（${tsKeyMap.defaultKey ?? "无"}）兜住`
        + " ⇒ 新增档忘记加 case 时，症状是那一档静默显示成「不确定」");
    }
    keyMap[token] = explicit ?? tsKeyMap.defaultKey;
  }
  // 键映射不完整时，后面的标签对账是在拿兜底键比真档位 ⇒ 无意义，直接收红返回
  if (keyIncomplete) { return bad; }

  // ── R1 前端 token 集合 == 权威 token 集合 ──
  const tsTokenList = Object.values(tsTokens);
  const rsTokenList = VARIANTS.map((v) => tokens[v]);
  if (!sameSet(tsTokenList, rsTokenList)) {
    const miss = rsTokenList.filter((t) => !tsTokenList.includes(t));
    const extra = tsTokenList.filter((t) => !rsTokenList.includes(t));
    push("R1", `前端 StockAction 与 ActionKind 值域不同集合：缺 ${miss.join("/") || "—"}，多 ${extra.join("/") || "—"}`);
  }

  // ── R2 权威中文 == zh-CN 标签（逐档同字） ──
  const zhValues = localeActionValues(parts.zhLocale, keyMap);
  for (const v of VARIANTS) {
    const token = tokens[v];
    const want = storageCn[v];
    const got = zhValues[token];
    if (got === undefined) { push("R2", `zh-CN 缺 ${v}（token ${token}）的标签键`); continue; }
    if (got !== want) { push("R2", `${v}：权威中文「${want}」≠ zh-CN「${got}」`); }
  }

  // ── R3 八键在 11 份 locale 齐备 ──
  for (const [locale, json] of Object.entries(parts.locales)) {
    const vals = localeActionValues(json, keyMap);
    for (const v of VARIANTS) {
      const got = vals[tokens[v]];
      if (!got) { push("R3", `${locale}.json 缺/空 ${keyMap[tokens[v]]}`); }
    }
  }

  // ── R4 非中英语言不得仍是英文 token（未译） ──
  for (const [locale, json] of Object.entries(parts.locales)) {
    if (locale === "en-US" || CJK_LOCALES.has(locale)) { continue; }
    const vals = localeActionValues(json, keyMap);
    const untranslated = VARIANTS
      .map((v) => tokens[v])
      .filter((t) => {
        const got = vals[t];
        return got !== undefined && isAsciiToken(got) && got.toUpperCase().replace(/\s+/g, "_") === t;
      });
    if (untranslated.length) {
      push("R4", `${locale}.json 的档位标签未译（显示英文原词）：${untranslated.join("/")}`);
    }
  }

  // ── R5 LLM schema 每处 enum 一致且等于六档操作建议 ──
  const operatingCn = OPERATING.map((v) => cn[v]).filter((x) => x !== null);
  const enums = schemaActionEnums(parts.schemaJson);
  if (!enums.length) { push("R5", "llm-output-schemas.json 里一个 action enum 也没解析到 ⇒ 判据失效"); }
  const copies = new Set();
  for (const e of enums) {
    copies.add(JSON.stringify(e.values));
    if (!sameSet(e.values, operatingCn)) {
      const miss = operatingCn.filter((x) => !e.values.includes(x));
      const extra = e.values.filter((x) => !operatingCn.includes(x));
      push("R5", `schema :${e.line} 的 enum 不等于权威六档：缺 ${miss.join("/") || "—"}，多 ${extra.join("/") || "—"}`);
    }
  }
  if (enums.length > 1 && copies.size > 1) {
    push("R5", `schema 里 ${enums.length} 处 action enum 彼此不一致（${[...copies].join(" vs ")}）`);
  }

  // ── R6 后端别名必须被前端严格解析表覆盖 ──
  const uncovered = aliases.filter((a) => !(a in tsLabels || a.toUpperCase() in tsTokens));
  if (uncovered.length) {
    push("R6", `后端 ACTION_ALIASES 有 ${uncovered.length} 项前端不认 ⇒ 同一条记录后端有档、界面显示「不确定」：${uncovered.join("/")}`);
  }

  // ── R7 产端不得输出别名档名（只允许权威中文） ──
  const canonicalCn = new Set(VARIANTS.map((v) => storageCn[v]));
  const aliasOnly = aliases.filter((a) => !isAsciiToken(a) && !canonicalCn.has(a));
  for (const [name, src] of Object.entries(parts.producers)) {
    const body = stripComments(src ?? "");
    const isMarkdown = /\.md$/.test(name);
    // 两种产面用两种打法，因为「出现别名」≠「输出别名」：
    //   · Rhai 决策脚本：档名只会以**带引号的字面量**产出，而 trail 文案里满是
    //     「禁止加仓」「不加仓」这类说明性用词 —— 按 token 扫会大面积误报（实测 6 处全在文案里）。
    //   · 提示词 md：档位清单以表格单元 / `"A | B | C"` 枚举形态出现，不带引号包裹单个词，
    //     故把 markdown 分隔符（`*`、反引号、`|`、引号、括号、顿号、斜杠）当边界切词。
    const hits = isMarkdown
      ? aliasOnly.filter((a) => new RegExp(`[\`*|、/「（"'\\[]\\s*${a}\\s*[\`*|、/」）"'\\]]`).test(body))
      : aliasOnly.filter((a) => body.includes(`"${a}"`));
    if (hits.length) {
      push("R7", `${name} 的输出清单含别名档名（应改权威中文；别名是解析入口不是档位）：${hits.join("/")}`);
    }
  }

  // ── R10 前端「枚举→中文」第二份表必须等于权威 as_storage_cn ──
  for (const v of VARIANTS) {
    const token = tokens[v];
    const got = tsCanonical[token];
    if (got === undefined) { push("R10", `STOCK_ACTION_CANONICAL_LABELS 缺 ${token}`); }
    else if (got !== storageCn[v]) { push("R10", `${token}：权威「${storageCn[v]}」≠ 前端表「${got}」`); }
  }
  const extraCanonical = Object.keys(tsCanonical).filter((t) => !rsTokenList.includes(t));
  if (extraCanonical.length) { push("R10", `前端表多出权威没有的档：${extraCanonical.join("/")}`); }

  // ── R11 强度配色：八档互异、六档 rank 恰好 0..5、顺序与权威谓词同向 ──
  const palette = tsPaletteMap(ts);
  if (!palette) {
    push("R11", "解析不到 ACTION_PALETTE ⇒ 配色判据失效，不得当通过");
  } else {
    const wantTokens = Object.keys(tsTokens);
    const missPal = wantTokens.filter((t) => !palette[t]);
    const extraPal = Object.keys(palette).filter((t) => !wantTokens.includes(t));
    if (missPal.length) { push("R11", `ACTION_PALETTE 缺档：${missPal.join("/")} ⇒ 该档无配色可查`); }
    if (extraPal.length) { push("R11", `ACTION_PALETTE 多出权威没有的档：${extraPal.join("/")}`); }

    // 八色互异 —— 用户 2026-10-22 的原话要求：「不要出现结果不同而颜色相同的情况」
    const byHex = {};
    for (const [t, v] of Object.entries(palette)) { (byHex[v.hex] ??= []).push(t); }
    for (const [hex, list] of Object.entries(byHex)) {
      if (list.length > 1) { push("R11", `${list.join("/")} 共用 hex ${hex} ⇒ 两档结论同色，强度差完全不可见`); }
    }

    // 六档 rank 恰好 0..5 各一次；两档兜底必须 null（缺席不摆上强度轴）
    const opRanks = OPERATING.map((v) => palette[tokens[v]]?.rank);
    const numeric = opRanks.filter((r) => typeof r === "number");
    if (numeric.length !== OPERATING.length) { push("R11", `六档里出现非数字 rank（缺席两档才允许 null）：${opRanks.join("/")}`); }
    if (new Set(numeric).size !== numeric.length) { push("R11", `六档 rank 有重复：${numeric.join("/")} ⇒ 无法按强度排出先后`); }
    const sorted = numeric.slice().sort((a, b) => a - b).join("/");
    if (sorted !== "0/1/2/3/4/5") { push("R11", `六档 rank 必须恰好是 0..5，实为 ${sorted || "空"}`); }
    for (const v of VARIANTS.filter((x) => cn[x] === null)) {
      const p = palette[tokens[v]];
      if (p && p.rank !== null) { push("R11", `${tokens[v]} 是兜底档（无操作语义），rank 必须为 null，实为 ${p.rank} ⇒ 缺席被摆上强度轴`); }
    }

    // 方向单调：利空档（implies_reduce）必须排在中性左侧，利好档（implies_buy）在右侧
    const reduceT = (rustPredicateSet(rs, "implies_reduce") ?? []).map((v) => tokens[v]);
    const buyT = (rustPredicateSet(rs, "implies_buy") ?? []).map((v) => tokens[v]);
    if (!reduceT.length || !buyT.length) {
      push("R11", "解析不到 implies_reduce / implies_buy ⇒ 方向单调性无法判");
    } else {
      const neutralT = OPERATING.map((v) => tokens[v]).filter((t) => !reduceT.includes(t) && !buyT.includes(t));
      const rankOf = (t) => palette[t]?.rank ?? Number.NaN;
      const bearMax = Math.max(...reduceT.map(rankOf));
      const neutMin = Math.min(...neutralT.map(rankOf));
      const neutMax = Math.max(...neutralT.map(rankOf));
      const bullMin = Math.min(...buyT.map(rankOf));
      if (!(bearMax < neutMin && neutMax < bullMin)) {
        push("R11", `配色强度序与权威谓词不同向：利空 ${reduceT.join("/")} 最大 rank ${bearMax}，`
          + `中性 ${neutralT.join("/")} ${neutMin}..${neutMax}，利好 ${buyT.join("/")} 最小 rank ${bullMin}`);
      }
    }
  }

  // ── R12 按档位分支只能住在权威文件里（第二份 switch = 改定义端静默漏改的老形态）──
  if (!parts.consumers) {
    push("R12", "没有扫描到前端消费面（parts.consumers 缺失）⇒ 判据失效，不得当通过");
  } else {
    for (const [rel, src] of Object.entries(parts.consumers)) {
      const code = stripComments(src);
      const sw = code.match(/case\s+StockAction\.[A-Z_]+\s*:/);
      if (sw) { push("R12", `${rel} 里出现按档位分支「${sw[0]}」⇒ 档位→色/键只允许一处（权威 ${FRONTEND_TS}）`); }
      const pal = code.match(/const\s+ACTION_PALETTE\b/);
      if (pal) { push("R12", `${rel} 里另立了一份 ACTION_PALETTE ⇒ 第二张配色表`); }
    }
  }

  // ── R13 记录来历值域（live / replay）单源：权威在 harness，消费端不得写字面量 ──
  // 消费端扫描**不依赖权威可解析**：权威文件搬家/改名时，既要报「解析不到」，也要把现存字面量
  // 一起列出来 —— 否则基线读数会被一条「解析不到」吞掉，看不出还要改多少处（首跑即此形态）。
  const rustHits = kindLiteralHits(parts.rustSources ?? {});
  if (rustHits.length) {
    push("R13", `Rust 侧 ${rustHits.length} 处把来历写成字面量（只能由权威给，测试夹具也在此列，注释已剥离）：`
      + `${rustHits.slice(0, 4).map((h) => ` ｜ ${h}`).join("")}${rustHits.length > 4 ? ` ｜ …另 ${rustHits.length - 4} 处` : ""}`);
  }
  const kindRs = parts.kindAuthorityRs ? stripComments(parts.kindAuthorityRs) : null;
  const kindStorage = kindRs ? rustVariantMap(kindRs, "as_storage") : null;
  const kindValues = kindRs ? rustStringArray(kindRs, "ANALYSIS_KIND_VALUES") : null;
  if (!kindRs || !kindStorage || !kindValues) {
    push("R13", `解析不到来历权威（${AUTHORITY_KIND_RS} 的 as_storage / ANALYSIS_KIND_VALUES）⇒ 判据失效，不得当通过`);
  } else {
    const storageVals = Object.values(kindStorage);
    if (!sameSet(storageVals, kindValues)) {
      push("R13", `权威内部不一致：as_storage 给 ${storageVals.join("/")}，ANALYSIS_KIND_VALUES 列 ${kindValues.join("/")}`);
    }
    const kindBody = functionBodyAfter(ts, "export function provenanceOfRecord");
    if (!kindBody) {
      push("R13", "解析不到前端 provenanceOfRecord ⇒ 来历的消费端不在门的扫描面上");
    } else {
      const tsVals = [...new Set([...kindBody.matchAll(/"([a-z_]+)"/g)].map((m) => m[1]))]
        .filter((v) => kindValues.includes(v));
      if (!sameSet(tsVals, kindValues)) {
        push("R13", `前端来历判据只认 ${tsVals.join("/") || "—"}，权威是 ${kindValues.join("/")}`
          + " ⇒ 新增一档会静默归「未知」，改定义端漏改消费端的老失效机制");
      }
    }
  }

  // ── R14 DB CHECK 的字面量清单必须逐档等于两个权威（值域的第三处表达 ⇒ 用门钉，不靠自觉）──
  const extras = parts.extrasRs ? stripComments(parts.extrasRs) : null;
  if (!extras) {
    push("R14", `解析不到 ${EXTRAS_RS} ⇒ DB 侧值域守卫不在门的扫描面上`);
  } else {
    const kindFromDb = sqlCheckArray(extras, "analysis_kind");
    if (!kindFromDb) {
      push("R14", "extras.rs 里没有 analysis_kind 的值域 CHECK ⇒ 只剩写侧守，旁路写入无人拦");
    } else if (kindValues && !sameSet(kindFromDb, kindValues)) {
      push("R14", `DB CHECK 的来历清单 ${kindFromDb.join("/")} ≠ 权威 ANALYSIS_KIND_VALUES ${kindValues.join("/")}`);
    }
    const actFromDb = sqlCheckArray(extras, "decision_action");
    const storageList = Object.values(storageCn);
    if (!actFromDb) {
      push("R14", "extras.rs 里没有 decision_action 的值域 CHECK ⇒ 同上");
    } else if (!sameSet(actFromDb, storageList)) {
      const miss = storageList.filter((v) => !actFromDb.includes(v));
      const extra = actFromDb.filter((v) => !storageList.includes(v));
      push("R14", `DB CHECK 的结论档清单与权威 as_storage_cn 不同：缺 ${miss.join("/") || "—"}，多 ${extra.join("/") || "—"}`);
    }
  }

  // ── R8 第二套同义 i18n 键 ──
  const zh = JSON.parse(parts.zhLocale);
  const dup = ["actionBuy", "actionSell", "actionReduce", "actionHold", "actionWait", "actionIncrease"]
    .filter((k) => zh.stockAnalysis?.conditional && k in zh.stockAnalysis.conditional);
  if (dup.length) {
    push("R8", `存在第二套同义档位键 stockAnalysis.conditional.*：${dup.join("/")}（应收编到顶层 action*）`);
  }

  return bad;
}

/**
 * R12 的扫描面：前端全部 `.ts/.tsx`（排测试、排 codegen 产物、排权威文件本身）。
 * 读盘一次进缓存；负控改的是 `parts.consumers` 这份**浅拷贝里的字符串**，不动缓存。
 */
let consumerCache = null;
function loadConsumers() {
  if (!consumerCache) {
    const out = {};
    const walk = (dir) => {
      let entries;
      try {
        entries = readdirSync(resolve(ROOT, dir), { withFileTypes: true });
      } catch {
        return;
      }
      for (const e of entries) {
        if (e.name.startsWith(".")) { continue; }
        const rel = `${dir}/${e.name}`;
        if (e.isDirectory()) {
          if (e.name === "__tests__" || e.name === "generated") { continue; }
          walk(rel);
        } else if (/\.tsx?$/.test(e.name) && rel !== FRONTEND_TS) {
          out[rel] = readFileSync(resolve(ROOT, rel), "utf8");
        }
      }
    };
    walk("src");
    consumerCache = out;
  }
  return { ...consumerCache };
}

/**
 * R13 的 Rust 消费面：`src-tauri/src` 与 `src-tauri/crates` 下全部 `.rs`。
 * 排除副本目录（`target` / `output` —— 本仓有整棵树的备份副本，算进扫描面会把引用判成零命中）
 * 与权威文件本身。
 */
let rustCache = null;
function loadRustSources() {
  if (!rustCache) {
    const out = {};
    const skipDirs = new Set(["target", "output", "fixtures"]);
    const walk = (dir) => {
      let entries;
      try {
        entries = readdirSync(resolve(ROOT, dir), { withFileTypes: true });
      } catch {
        return;
      }
      for (const e of entries) {
        if (e.name.startsWith(".")) { continue; }
        const rel = `${dir}/${e.name}`;
        if (e.isDirectory()) {
          if (skipDirs.has(e.name)) { continue; }
          walk(rel);
        } else if (e.name.endsWith(".rs") && rel !== AUTHORITY_KIND_RS) {
          out[rel] = readFileSync(resolve(ROOT, rel), "utf8");
        }
      }
    };
    walk("src-tauri/src");
    walk("src-tauri/crates");
    rustCache = out;
  }
  return { ...rustCache };
}

/**
 * R13 的判据：一行代码里**既有** analysis_kind 相关标识符、**又有**裸的 `"live"` / `"replay"`
 * 字面量 ⇒ 红（值域只能由权威给）。剥注释后再判 —— 实体注释里写 `'live' | 'replay' | 'ab_test'`
 * 这类散文不是消费点，把它算进去就是门自己的假红。
 */
function kindLiteralHits(rsSources) {
  const hits = [];
  for (const [rel, src] of Object.entries(rsSources)) {
    const code = stripComments(src);
    code.split("\n").forEach((line, i) => {
      if (!/analysis_kind|AnalysisKind/.test(line)) { return; }
      const lit = line.match(/"(live|replay)"/);
      if (lit) { hits.push(`${rel}:${i + 1}  ${line.trim().slice(0, 90)}`); }
    });
  }
  return hits;
}

/**
 * TS 函数体：从 `sig`（**不含左括号**）起，按括号深度跳过参数表里的内联对象类型，
 * 取第一个「不在括号内」的 `{` 作为函数体起点。
 *
 * ⚠ 不能复用 [`bodyAfter`] 直接找第一个 `{`：`provenanceOfRecord(r: { analysisKind?: … })`
 * 的参数类型自带花括号，会把参数当成函数体（palette 那条也栽过同一形态）。
 */
export function functionBodyAfter(src, sig) {
  const at = src.indexOf(sig);
  if (at < 0) { return null; }
  let paren = 0;
  for (let i = at; i < src.length; i += 1) {
    const c = src[i];
    if (c === "(") { paren += 1; } else if (c === ")") { paren -= 1; } else if (c === "{" && paren === 0) {
      let depth = 0;
      for (let j = i; j < src.length; j += 1) {
        if (src[j] === "{") { depth += 1; } else if (src[j] === "}") {
          depth -= 1;
          if (depth === 0) { return src.slice(i, j + 1); }
        }
      }
      return null;
    }
  }
  return null;
}

/**
 * R14：DB 侧 `CHECK` 约束里的 SQL 值域清单（`ARRAY['a','b']` 形态）—— 取某列那条的取值数组。
 * 找不到返回 null（调用方按红处理，不得当「没有约束 = 通过」）。
 */
export function sqlCheckArray(extrasSrc, colName) {
  const re = new RegExp(`${colName}[^\\n]*?ARRAY\\[([^\\]]*)\\]`);
  const m = extrasSrc.match(re);
  if (!m) { return null; }
  const items = [...m[1].matchAll(/'([^']*)'/g)].map((x) => x[1]);
  return items.length ? items : null;
}

function loadParts() {
  const missing = [];
  const readOpt = (rel) => {
    try {
      return read(rel);
    } catch {
      missing.push(rel);
      return null;
    }
  };
  const locales = {};
  for (const l of LOCALES) { locales[l] = readOpt(`src/i18n/locales/${l}.json`); }
  const producers = {};
  for (const p of [...RHAI_SOURCES, ...PROMPT_SOURCES]) { producers[p] = readOpt(p); }
  return {
    authorityRs: readOpt(AUTHORITY_RS),
    kindAuthorityRs: readOpt(AUTHORITY_KIND_RS),
    extrasRs: readOpt(EXTRAS_RS),
    frontendTs: readOpt(FRONTEND_TS),
    schemaJson: readOpt(LLM_SCHEMA_JSON),
    zhLocale: locales["zh-CN"],
    locales,
    producers,
    consumers: loadConsumers(),
    rustSources: loadRustSources(),
    missing,
  };
}

function byRule(bad) {
  const g = {};
  for (const b of bad) { (g[b.rule] ??= []).push(b); }
  return g;
}

// ── 负控：每条判据一个破坏样本，断言「被破坏那一格对应的那条红、且只红那一条」 ──
/** 改 locale JSON 一律走 parse→改→stringify（字符串替换容易造出非法 JSON 而假红成「解析崩」） */
function editLocale(parts, lang, fn) {
  const data = JSON.parse(parts.locales[lang]);
  fn(data.stockAnalysis);
  const dumped = JSON.stringify(data, null, 2) + "\n";
  parts.locales[lang] = dumped;
  if (lang === "zh-CN") { parts.zhLocale = dumped; }
}

function selftest() {
  const cases = [
    // 破坏样本用「前端多一档」而不是「删一档」：删 WAIT 会连带让别名 `wait` 失去覆盖（R6 也真该红），
    // 那是跨规则的正确联动，不能拿它当「本条判据无电」，也不能让负控误判为串味
    ["R1", (p) => { p.frontendTs = p.frontendTs.replace('  HOLD: "HOLD",\n', '  HOLD: "HOLD",\n  SUPER_HOLD: "SUPER_HOLD",\n'); }, ["R11"]],
    ["R2", (p) => { editLocale(p, "zh-CN", (sa) => { sa.actionReduce = "减仓"; }); }],
    ["R3", (p) => { editLocale(p, "ko", (sa) => { delete sa.actionUnavailable; }); }],
    ["R4", (p) => { editLocale(p, "de", (sa) => { sa.actionWait = "WAIT"; }); }],
    ["R5", (p) => { p.schemaJson = p.schemaJson.replace('            "观望",\n', ""); }],
    ["R6", (p) => { p.frontendTs = p.frontendTs.replace('  "加仓": StockAction.INCREASE,\n', ""); }],
    ["R7", (p) => { p.producers[RHAI_SOURCES[0]] = p.producers[RHAI_SOURCES[0]].replace(/"观望"/g, '"等待"'); }],
    // R7 的提示词面单独一条：证明 md 不再因「清单不带引号」而恒不命中（上一版的假覆盖面）
    ["R7", (p) => { p.producers[PROMPT_SOURCES[0]] += "\n\n| 动作 | 减仓 |\n"; }],
    ["R8", (p) => { editLocale(p, "zh-CN", (sa) => { sa.conditional.actionHold = "持有"; }); }],
    ["R9", (p) => { p.frontendTs = p.frontendTs.replace('      return "stockAnalysis.actionSell";\n', ""); }],
    ["R10", (p) => { p.frontendTs = p.frontendTs.replace('[StockAction.HOLD]: "持有"', '[StockAction.HOLD]: "坚守"'); }],
    // R11 的四格各拆一个破坏样本：同色 / rank 重复 / 兜底档被摆上强度轴 / 强度方向反了
    ["R11", (p) => { p.frontendTs = p.frontendTs.replace('[StockAction.HOLD]: { hex: "#faad14", rank: 3 }', '[StockAction.HOLD]: { hex: "#f5222d", rank: 3 }'); }],
    ["R11", (p) => { p.frontendTs = p.frontendTs.replace('[StockAction.HOLD]: { hex: "#faad14", rank: 3 }', '[StockAction.HOLD]: { hex: "#faad14", rank: 1 }'); }],
    ["R11", (p) => { p.frontendTs = p.frontendTs.replace('[StockAction.UNCERTAIN]: { hex: "#9254de", rank: null }', '[StockAction.UNCERTAIN]: { hex: "#9254de", rank: 6 }'); }],
    ["R11", (p) => {
      p.frontendTs = p.frontendTs
        .replace('[StockAction.SELL]: { hex: "#13c2c2", rank: 0 }', '[StockAction.SELL]: { hex: "#13c2c2", rank: 5 }')
        .replace('[StockAction.BUY]: { hex: "#f5222d", rank: 5 }', '[StockAction.BUY]: { hex: "#f5222d", rank: 0 }');
    }],
    // R12：在某个消费组件里另立按档位分支（八色互异仍成立，只有这条该红）
    ["R12", (p) => {
      const k = Object.keys(p.consumers).find((rel) => rel.endsWith("InvestDashboard.tsx"));
      p.consumers[k] += "\nfunction probe(a: string) { switch (a) { case StockAction.BUY: return 1; default: return 0; } }\n";
    }],
    // R13 三格：消费端写字面量 / 前端判据少认一档 / 权威内部两张清单不一致
    ["R13", (p) => {
      const k = Object.keys(p.rustSources).find((rel) => rel.endsWith("pool_scan.rs"));
      p.rustSources[k] += '\nfn probe() { let _ = stock_analyses::Column::AnalysisKind.eq("replay"); }\n';
    }],
    ["R13", (p) => { p.frontendTs = p.frontendTs.replace('if (kind === "live") { return "live"; }', ""); }],
    ["R13", (p) => {
      if (!p.kindAuthorityRs) { throw new Error("来历权威 analysis_kind.rs 尚未落地 ⇒ 这条负控要等 R-DA-7① 做完才能验"); }
      p.kindAuthorityRs = p.kindAuthorityRs.replace('Self::Replay => "replay"', 'Self::Replay => "backtest"');
    }],
    // R14 两格：DB CHECK 少一档来历 / 结论档清单漂一个词
    ["R14", (p) => { p.extrasRs = p.extrasRs.replace("ARRAY['live','replay']", "ARRAY['live']"); }],
    ["R14", (p) => { p.extrasRs = p.extrasRs.replace("'不确定','数据缺失'", "'不确定','缺失'"); }],
  ];
  const clean = byRule(checkAll(loadParts()));
  const baseline = Object.fromEntries(Object.keys(clean).map((r) => [r, clean[r].map((b) => b.msg)]));
  let fail = 0;
  for (const entry of cases) {
    const [rule, mutate, alsoAllow = []] = entry;
    const parts = loadParts();
    mutate(parts);
    const after = byRule(checkAll(parts));
    // 比对**消息集合**而不是条数：一条规则对多份 locale 只出一行，
    // 改一个已经红的样本不会加条数（拿条数判负控会把有效破坏样本读成「无电」）
    const newMsgs = (after[rule] ?? []).map((b) => b.msg).filter((m) => !(baseline[rule] ?? []).includes(m));
    const hit = newMsgs.length > 0;
    const others = Object.keys(after).filter((r) => r !== rule
      && r !== "R0"
      && !alsoAllow.includes(r)
      && (after[r] ?? []).some((b) => !(baseline[r] ?? []).includes(b.msg)));
    const ok = hit && others.length === 0;
    if (!ok) { fail += 1; }
    console.log(`${ok ? "OK  " : "FAIL"} ${rule} 负控：新增红 ${hit ? `有（${newMsgs[0]?.slice(0, 40)}…）` : "无"}`
      + (others.length ? `，但连带触红 ${others.join("/")}` : ""));
  }
  console.log(fail === 0 ? `\n负控结论: ${cases.length} 个破坏样本各自有电` : `\n负控结论: ${fail} 个样本失效`);
  return fail === 0;
}

/** 只在**直接执行**时跑门禁；被 import（测试 / 其它脚本复用判据）时不产生副作用 */
const invokedDirectly = typeof process.argv[1] === "string"
  && pathToFileURL(process.argv[1]).href === import.meta.url;

if (invokedDirectly) {
  const args = process.argv.slice(2);
  if (args.includes("--selftest")) {
    process.exit(selftest() ? 0 : 1);
  }
  if (args.includes("--dump")) {
    const parts = loadParts();
    const rs = stripComments(parts.authorityRs);
    const ts = stripComments(parts.frontendTs);
    console.log("as_token      ", JSON.stringify(rustVariantMap(rs, "as_token")));
    console.log("as_cn         ", JSON.stringify(rustVariantMap(rs, "as_cn")));
    console.log("as_storage_cn ", JSON.stringify(rustVariantMap(rs, "as_storage_cn")));
    console.log("ACTION_ALIASES", JSON.stringify(rustStringArray(rs, "ACTION_ALIASES")));
    console.log("StockAction   ", JSON.stringify(tsConstMap(ts, "StockAction")));
    console.log("TS labels     ", JSON.stringify(tsLabelMap(ts)));
    console.log("TS canonical  ", JSON.stringify(tsCanonicalLabelMap(ts)));
    console.log("TS palette    ", JSON.stringify(tsPaletteMap(ts)));
    console.log("TS i18n keys  ", JSON.stringify(tsActionKeyMap(ts)));
    console.log("schema enums  ", JSON.stringify(schemaActionEnums(parts.schemaJson)));
    const krs = parts.kindAuthorityRs ? stripComments(parts.kindAuthorityRs) : "";
    console.log("kind storage  ", JSON.stringify(krs ? rustVariantMap(krs, "as_storage") : null));
    console.log("kind values   ", JSON.stringify(krs ? rustStringArray(krs, "ANALYSIS_KIND_VALUES") : null));
    console.log("kind literal sites in Rust", String(kindLiteralHits(parts.rustSources ?? {}).length));
    process.exit(0);
  }

  const bad = checkAll(loadParts());
  const grouped = byRule(bad);
  for (const rule of Object.keys(grouped).sort()) {
    console.log(`\n${rule} —— ${grouped[rule].length} 处`);
    for (const b of grouped[rule]) { console.log(`  · ${b.msg}`); }
  }
  if (!bad.length) { console.log("OK  决策结论档八面对账一致"); }
  else { console.log(`\n结论: ${bad.length} 处违规（门未全绿前不接入 CI）`); }
  process.exit(bad.length ? 1 : 0);
}
