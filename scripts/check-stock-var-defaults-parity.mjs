#!/usr/bin/env node
/**
 * 股票分析模板变量「默认值」等式门禁（面板兜底值 ↔ 后端种子值）。
 *
 * ## 为什么需要它
 *
 * `stock-analysis` 模板变量的出厂默认值有**两处载体**，且两处都会落到库里：
 *
 * | 载体 | 位置 | 何时生效 |
 * |---|---|---|
 * | **权威源** | `src-tauri/src/commands/stock_analysis_setup/seed_variables.rs` 的 `Variable { value: serde_json::json!(…) }` | 播种 / 换代时写进模板 `variables` |
 * | 前端兜底 | `src/components/settings/StockAnalysisConfigPanel.tsx` 的 `getDefaultVariables()` 里 `b(name, 值, 描述键, 类型)` | ① 模板 `variables` 为空时**当作初值写回库**；② 面板取不到后端值时的显示兜底；③ `WhatIfBacktest` 的滑块默认值与标签 |
 *
 * 两侧一旦不等，「跑出来的数取决于走了哪条初始化路径」—— 本仓已实证过两次：
 * · `risk_max_drawdown_limit` 后端 20 / 面板 15（见 `getDefaultVariables()` 上方注释）；
 * · `debate_rounds` 面板 1 / 后端 3（2026-10-12 v154，v161 又改回 1）。
 * 第二次补的锁（`StockAnalysisConfigPanel.test.tsx` 的「debate_rounds 默认值同源」）只钉住
 * **那一条**，其余 130+ 条照样能各改各的。本门把它升成**全表**。
 *
 * ## 与既有三道门的边界（不是重复）
 *
 * - `check-variable-consumer-registry.mjs`：管「声明的变量**有没有**消费者」（面积上限，不比值）。
 * - `check-panel-var-landing.mjs`：管**已接线那 15 条**的三面值等式（种子 ≡ 面板 ≡ 落点回落值）。
 * - `check-valuation-defaults-parity.mjs`：管估值 DTO 那 7 个字段横跨 5 处载体（另一套载体，
 *   与本门的 `b()` 清单不相交）。
 * - **本门**：管**其余全部** `b()` 兜底值 ↔ 种子声明值的**值 / 键 / 类型三面对账**。
 *   新增 `b()` 条目必须同批在种子里声明，否则红 —— 「同一属性两处设置」正是本形态。
 *
 * ## 判定规则
 *
 * - **V1 值等式**：面板 `b()` 的值 ≡ 种子 `value` 求值结果。数字用相对容差 `1e-9`
 *   —— 种子允许写**派生式**（`DEFAULT_DCF_DISCOUNT_RATE_PCT = …::DISCOUNT_RATE * 100.0`，
 *   IEEE754 下 `0.077 * 100 = 7.700000000000001`），把它判成「不等」是门禁的假阳而不是缺陷；
 *   两侧都是十进制字面量时容差永远不触发，仍等价于严格相等。布尔与字符串严格相等。
 * - **V2 键存在**：面板声明的每个键必须在种子变量表里存在。缺席 ⇒ 红。
 * - **V3 类型等式**：`b()` 的类型参数 ≡ 种子的 `var_type`（现网 0 处漂移 ⇒ 钉住；
 *   类型漂移会让滑杆量程与真实值域打架）。
 * - **V4 求值面**：种子 `value` 必须可解析 —— 字面量 / 同文件 `pub const` / 跨 crate
 *   `axagent_*::*::CONST`（任意深度派生链，共用 `scripts/lib/rust-const-expr.mjs`）。
 *   解不出 ⇒ 按**解析失败**（exit 2），不当成 0、也不跳过。
 * - **V5 判据没电自证**：任一侧抓不到、条数低于下限，或 `b(` 调用数 ≠ 解析出的条数
 *   ⇒ exit 2。「0 命中 ≠ 没问题」（判据 #7），「抓到一部分但漏了一种写法」同样不等于通过。
 *
 * ⚠️ 本门**不比**「种子有而面板没有」的方向：面板刻意不暴露某些键（`signal_rsi_*` 两条无控件、
 *    `vendor_*` 由 `DataVendorsTab` 全权管理），那是设计而非漂移。
 *
 * ## 用法
 *
 *   node scripts/check-stock-var-defaults-parity.mjs             # 正式门禁
 *   node scripts/check-stock-var-defaults-parity.mjs --selftest  # 正负对照（不读工作区）
 *   node scripts/check-stock-var-defaults-parity.mjs --dump      # 逐条读数（值 + 求值来源）
 *
 * 退出码：0 通过 ｜ 1 等式不符 / 键缺席 ｜ 2 解析失败（脚本自身失效，写法变了）
 */
import { readFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { evalConstExpr, resolveConstsToFixedPoint } from "./lib/rust-const-expr.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");

const SEED_VARS = "src-tauri/src/commands/stock_analysis_setup/seed_variables.rs";
const PANEL = "src/components/settings/StockAnalysisConfigPanel.tsx";

/** 解析面下限：现网种子 357 条 / 面板 134 条 —— 低于下限即判「脚本失效」，不当「无违规」 */
const MIN_SEED_VARS = 250;
const MIN_PANEL_VARS = 100;

/** 数字容差：只为「派生式 ×100.0」的浮点尾差放行，字面量之间仍等价于严格相等 */
const NUM_EPS = 1e-9;

/**
 * 依赖文件读取器。正式门禁读工作区；`--selftest` 注入内存样本，
 * 于是跨 crate 求值路径也能被合成样本覆盖（不把自检绑在真实 `mcp_tools.rs` 的内容上）。
 */
let readDep = (rel) => readFileSync(path.join(ROOT, rel), "utf8");

/** 已解析过的依赖文件 → 常量值 Map（每轮 evaluate 前清空，避免自检样本互相污染） */
const constFileCache = new Map();

/** 跨 crate 常量路径：`axagent_astock_data::mcp_tools::DISCOUNT_RATE` */
const QUALIFIED_RE = /\b(axagent_[a-z0-9_]+(?:::[A-Za-z0-9_]+)+::[A-Z][A-Z0-9_]*)/g;

/** `axagent_analysis_engine::horizon_prior` → `src-tauri/crates/analysis-engine/src/horizon_prior.rs` */
function crateFileForModule(pkg, modPath) {
  if (!pkg.startsWith("axagent_")) {
    return null;
  }
  const crateDir = pkg.slice("axagent_".length).replace(/_/g, "-");
  const modFile = modPath.replace(/::/g, "/");
  for (const rel of [
    `src-tauri/crates/${crateDir}/src/${modFile}.rs`,
    `src-tauri/crates/${crateDir}/src/${modFile}/mod.rs`,
  ]) {
    try {
      readDep(rel);
      return rel;
    } catch {
      // 换一个候选路径继续试
    }
  }
  return null;
}

/** 同文件 `pub const NAME: T = expr;` 抽取（含 `pub(crate)`） */
function extractConstDefs(src) {
  const defs = new Map();
  for (
    const m of src.matchAll(
      /^\s*pub(?:\([a-z]+\))?\s+const\s+([A-Z][A-Z0-9_]*)\s*:\s*[^=;]+=\s*([^;]+);/gm,
    )
  ) {
    defs.set(m[1], m[2].trim());
  }
  return defs;
}

/** 值形态分类：`pub const` 的字面量右值 ⇒ `{kind, value}`；不是字面量 ⇒ null */
function classifyLiteral(t) {
  if (/^-?[\d][\d_]*(?:\.\d[\d_]*)?$/.test(t)) {
    return { kind: "num", value: Number(t.replace(/_/g, "")) };
  }
  if (t === "true" || t === "false") {
    return { kind: "bool", value: t === "true" };
  }
  const s = /^"([^"]*)"$/.exec(t);
  if (s) {
    return { kind: "str", value: s[1] };
  }
  return null;
}

/**
 * 把表达式里的 `axagent_x::mod::CONST` 就地换成 `(数值)`；解不出的原样保留，
 * 交给 `evalConstExpr` 判为不可解析（**不**当成 0）。
 */
function inlineQualified(expr, chain) {
  return expr.replace(QUALIFIED_RE, (full) => {
    const hit = splitQualified(full);
    if (!hit) {
      return full;
    }
    const v = fileConsts(hit.rel, chain).nums.get(hit.name);
    return v === undefined ? full : `(${v})`;
  });
}

/** `axagent_a::m::NAME` ⇒ `{rel, name}`；定位不到源文件 ⇒ null */
function splitQualified(full) {
  const parts = full.split("::");
  const name = parts.pop();
  const rel = crateFileForModule(parts[0], parts.slice(1).join("::"));
  return rel ? { rel, name } : null;
}

/**
 * 某源文件的常量表：`defs` 是原始右值文本（限定路径已就地展开成数字，便于算术求值），
 * `nums` 是这些定义按依赖顺序求出的**数值**结果（不动点）。
 *
 * ⚠ 需要两张表而不能只留 `nums`：现网有 `pub(crate) const DEFAULT_RECO_IC_GATE: &str = "on"`
 *   —— 字符串常量参与算术必然求值失败，只按数值表查就会把它误判成「不可解析」（V4 假红）。
 *
 * `chain` 是本轮已读过的文件，用于切循环引用（A→B→A）；命中循环时该常量当作不可解析。
 */
function fileConsts(rel, chain = []) {
  if (constFileCache.has(rel)) {
    return constFileCache.get(rel);
  }
  if (chain.includes(rel)) {
    return { defs: new Map(), nums: new Map() };
  }
  let src = "";
  try {
    src = readDep(rel);
  } catch {
    return { defs: new Map(), nums: new Map() };
  }
  const nextChain = [...chain, rel];
  const defs = extractConstDefs(src);
  for (const [name, expr] of defs) {
    defs.set(name, inlineQualified(expr, nextChain));
  }
  const entry = { defs, nums: resolveConstsToFixedPoint(defs) };
  constFileCache.set(rel, entry);
  return entry;
}

const MAX_CONST_HOPS = 8;

/**
 * 把一个 Rust 右值文本求成 `{kind, value, source}`。
 * 依次尝试：字面量 → 整条限定路径（跨 crate，按该文件定义的**形态**递归）→
 * 整条同文件标识符（递归；字符串 / 布尔常量走这一条）→ 算术式（含混合限定路径）。
 * 全部失败 ⇒ `kind = "unresolved"`，由调用方计入**解析失败**（exit 2），绝不当成 0。
 */
function resolveExprText(text, defs, chain, hops = 0) {
  const t = text.trim();
  const lit = classifyLiteral(t);
  if (lit) {
    return { ...lit, source: "字面量" };
  }
  if (hops >= MAX_CONST_HOPS) {
    return { kind: "unresolved", value: null, source: `${t}（常量链超过 ${MAX_CONST_HOPS} 跳）` };
  }
  // ⚠ 只认「整条就是一个限定路径」的形态（`A * 100.0` 这类混合式留给后面的算术分支，
  //   否则会被当成「查不到的常量名」而误判不可解析）。
  const qFull = /^(axagent_[a-z0-9_]+(?:::[A-Za-z0-9_]+)+)::([A-Z][A-Z0-9_]*)$/.exec(t);
  if (qFull) {
    const parts = qFull[1].split("::");
    const rel = crateFileForModule(parts[0], parts.slice(1).join("::"));
    if (!rel) {
      return { kind: "unresolved", value: null, source: t };
    }
    const dep = fileConsts(rel, chain);
    const defText = dep.defs.get(qFull[2]);
    if (defText === undefined) {
      return { kind: "unresolved", value: null, source: t };
    }
    return resolveExprText(defText, dep.defs, [...chain, rel], hops + 1);
  }
  if (/^[A-Za-z_][A-Za-z0-9_]*$/.test(t) && defs.has(t)) {
    return resolveExprText(defs.get(t), defs, chain, hops + 1);
  }
  const inlined = inlineQualified(t, chain);
  const v = evalConstExpr(inlined, resolveConstsToFixedPoint(defs));
  if (v !== null) {
    return { kind: "num", value: v, source: inlined === t ? `派生自 ${t}` : `派生式 ${inlined}` };
  }
  return { kind: "unresolved", value: null, source: t };
}

/** 种子值表达式 → `{kind, value, source}`；不可解析 ⇒ `kind = "unresolved"` */
export function resolveSeedValue(expr, seedConsts) {
  return resolveExprText(expr, seedConsts, [], 0);
}

/**
 * 种子：抓全部 `Variable { name, var_type, value }`。
 * 三个字段**各抓各的**（块内允许夹注释，见现网 `reco_min_confidence` / `action_hold_threshold`），
 * 整块再按 `\n<缩进>},` 收尾 —— 与真实写法一致，写法变了会被 V5 的 `missShape` 逼红。
 */
export function parseSeed(src) {
  const rows = [];
  const missShape = [];
  for (const m of src.matchAll(/Variable\s*\{([\s\S]*?)\n\s*\},/g)) {
    const block = m[1];
    const name = /name:\s*"([a-z_][a-z0-9_]*)"\s*\.into\(\)/.exec(block);
    const type = /var_type:\s*"([a-z_]+)"\s*\.into\(\)/.exec(block);
    // `json!(…)` 的右括号后面可能是 `,`、换行，或就是块尾（`value` 是最后一个字段时，
    // 块正则已把 `\n},` 吃掉）⇒ 三种都得认，否则「写法没变、只是位置不同」会被判成形态未识别。
    const value = /value:\s*serde_json::json!\(\s*([\s\S]*?)\s*\)(?:,|\n|$)/.exec(block);
    if (!name || !type || !value) {
      missShape.push(
        `${name ? name[1] : "<无名>"}（缺 ${[!name && "name", !type && "var_type", !value && "value"].filter(Boolean).join("/")}）`,
      );
      continue;
    }
    rows.push({ name: name[1], type: type[1], expr: value[1].trim() });
  }
  return { rows, missShape };
}

/** 面板：只取 `getDefaultVariables()` 函数体内的 `b(name, 值, 描述键, 类型)` 声明 */
export function parsePanel(src) {
  const anchor = src.indexOf("export function getDefaultVariables");
  if (anchor < 0) {
    return { rows: [], callCount: 0, missingAnchor: true };
  }
  const braceStart = src.indexOf("{", anchor);
  let depth = 0;
  let end = -1;
  for (let i = braceStart; i < src.length; i++) {
    const c = src[i];
    if (c === "{") {
      depth++;
    } else if (c === "}") {
      depth--;
      if (depth === 0) {
        end = i;
        break;
      }
    }
  }
  if (end < 0) {
    return { rows: [], callCount: 0, missingAnchor: true };
  }
  const body = src.slice(braceStart, end);
  const rows = [];
  for (
    const m of body.matchAll(
      /b\(\s*"([a-z_][a-z0-9_]*)"\s*,\s*([^,\n]+?)\s*,\s*"([^"]*)"\s*,\s*"([^"]*)"\s*,?\s*\)/g,
    )
  ) {
    rows.push({ name: m[1], literal: m[2].trim(), type: m[4] });
  }
  return { rows, callCount: (body.match(/\bb\(/g) ?? []).length, missingAnchor: false };
}

function panelValue(literal) {
  if (/^-?[\d][\d_]*(?:\.\d[\d_]*)?(?:[eE][-+]?\d+)?$/.test(literal)) {
    const n = Number(literal.replace(/_/g, ""));
    if (Number.isFinite(n)) {
      return { kind: "num", value: n };
    }
  }
  if (literal === "true" || literal === "false") {
    return { kind: "bool", value: literal === "true" };
  }
  const s = /^"([^"]*)"$/.exec(literal);
  if (s) {
    return { kind: "str", value: s[1] };
  }
  return { kind: "unknown", value: null };
}

function numEq(a, b) {
  if (a === b) {
    return true;
  }
  const scale = Math.max(1, Math.abs(a), Math.abs(b));
  return Math.abs(a - b) <= NUM_EPS * scale;
}

/**
 * 汇总判定。
 *
 * @param {{seedSrc: string, panelSrc: string, readDepFile?: (rel: string) => string}} input
 *        `readDepFile` 仅供 `--selftest` 注入合成依赖文件；正式门禁省略 ⇒ 读工作区。
 * @returns {{rows: Array, parseErrors: string[], errors: string[]}}
 */
export function evaluate({ seedSrc, panelSrc, readDepFile }) {
  readDep = readDepFile ?? ((rel) => readFileSync(path.join(ROOT, rel), "utf8"));
  constFileCache.clear();

  const pErr = [];
  const err = [];

  const seed = parseSeed(seedSrc);
  if (seed.rows.length < MIN_SEED_VARS) {
    pErr.push(
      `[V5 种子] 只抓到 ${seed.rows.length} 条变量（下限 ${MIN_SEED_VARS}）—— ` +
        `写法变了或解析器失效，本门的「通过」不可信`,
    );
  }
  for (const s of seed.missShape) {
    pErr.push(`[V5 种子] \`Variable { … }\` 块形态未识别：${s}`);
  }

  const panel = parsePanel(panelSrc);
  if (panel.missingAnchor) {
    pErr.push("[V5 面板] 未定位 `export function getDefaultVariables()` 的函数体");
  } else if (panel.rows.length < MIN_PANEL_VARS) {
    pErr.push(
      `[V5 面板] 只抓到 ${panel.rows.length} 条 \`b()\` 声明（下限 ${MIN_PANEL_VARS}）—— 判据没电`,
    );
  } else if (panel.callCount !== panel.rows.length) {
    pErr.push(
      `[V5 面板] \`b("\` 调用 ${panel.callCount} 处，只解析出 ${panel.rows.length} 条 ⇒ ` +
        `存在本门读不到的第三种写法（静默漏检比红更危险）`,
    );
  }

  // 种子自身的常量表（同文件 `pub const` 右值文本）。限定路径不在这里展开也行
  // （`resolveExprText` 会按需跨文件递归），展开只是为了让算术分支直接拿到数值。
  const seedConsts = extractConstDefs(seedSrc);
  for (const [name, expr] of [...seedConsts]) {
    seedConsts.set(name, inlineQualified(expr, [SEED_VARS]));
  }

  const seedByName = new Map();
  for (const r of seed.rows) {
    const v = resolveSeedValue(r.expr, seedConsts);
    if (v.kind === "unresolved") {
      pErr.push(
        `[V4 种子] \`${r.name}\` 的值无法求值：\`${r.expr}\` —— ` +
          `须为字面量 / 同文件 \`pub const\` / 可定位的 \`axagent_*::*::CONST\`` +
          `（引用不存在的常量也算失败，不当成 0）`,
      );
    }
    seedByName.set(r.name, { ...r, resolved: v });
  }

  const rows = [];
  for (const p of panel.rows) {
    const pv = panelValue(p.literal);
    const s = seedByName.get(p.name);
    const row = {
      name: p.name,
      panel: p.literal,
      seed: s ? s.expr : "<种子无此键>",
      seedValue: s ? s.resolved.value : null,
      ok: false,
    };
    if (!s) {
      err.push(
        `[V2 键存在] 面板 \`b("${p.name}", …)\` 声明了默认值，种子变量表里却没有这条 —— ` +
          `「同一属性两处可设」正是本门要拦的形态：要么在 \`seed_variables.rs\` 声明，要么撤下面板这一条`,
      );
      rows.push(row);
      continue;
    }
    row.ok = true;
    if (p.type !== s.type) {
      row.ok = false;
      err.push(`[V3 类型] \`${p.name}\` 面板=${p.type}，种子 var_type=${s.type}`);
    }
    if (pv.kind === "unknown") {
      pErr.push(`[V5 面板] \`${p.name}\` 的默认值写法本门读不到：\`${p.literal}\``);
      row.ok = false;
    } else if (s.resolved.kind === "unresolved") {
      row.ok = false; // 已计入 V4 解析失败，不重复报等式错
    } else if (pv.kind !== s.resolved.kind) {
      row.ok = false;
      err.push(
        `[V1 值等式] \`${p.name}\` 值类型不符：面板=${pv.kind}(${p.literal})，` +
          `种子=${s.resolved.kind}(${s.resolved.value})`,
      );
    } else {
      const eq = pv.kind === "num"
        ? numEq(pv.value, s.resolved.value)
        : pv.value === s.resolved.value;
      row.ok = row.ok && eq;
      if (!eq) {
        err.push(
          `[V1 值等式] \`${p.name}\` 不相等：面板=${pv.value}，种子=${s.resolved.value}` +
            `（种子表达式 \`${s.expr}\`，求值来源：${s.resolved.source}）`,
        );
      }
    }
    rows.push(row);
  }

  return { rows, parseErrors: pErr, errors: err };
}

// ── 自检：正负对照（纯合成样本，跨 crate 路径也走注入的内存文件） ──
function selftest() {
  const seedBlock = (name, type, valueExpr) =>
    `Variable {\n    name: "${name}".into(),\n    var_type: "${type}".into(),\n` +
    `    value: serde_json::json!(${valueExpr}),\n    description: Some("d".into()),\n},`;

  /** 种子样本：4 个核心变量 + 300 条填充（填过 V5 的 250 下限，且不参与断言） */
  const seedSrc = (core, consts = "") =>
    consts + core.join("\n") + "\n" +
    Array.from({ length: 300 }, (_, i) => seedBlock(`pad_${i}`, "number", "1")).join("\n");

  const PANEL_HEAD = [
    "export function getDefaultVariables(): Variable[] {",
    "  const vars: Variable[] = [];",
    "  const b = (name: string, val: unknown, desc: string, type: string) =>",
    "    vars.push({ name, varType: type, value: val, description: desc, isSecret: false });",
  ];
  const PANEL_TAIL = ["  return vars;", "}"];
  /** 面板样本：4 个核心声明 + 120 条填充（与种子 `pad_*` 同键 ⇒ 值等式天然成立） */
  const panelSrc = (coreDecls) =>
    [
      ...PANEL_HEAD,
      ...coreDecls,
      ...Array.from({ length: 120 }, (_, i) => `  b("pad_${i}", 1, "p", "number");`),
      ...PANEL_TAIL,
    ].join("\n");

  const CORE_SEED = [
    seedBlock("max_concurrent", "number", "DEFAULT_X"),
    // 注释夹在字段之间也必须抽出来（现网 `reco_min_confidence` 就是这个形态）
    `Variable {\n    name: "agent_temperature".into(),\n    var_type: "number".into(),\n` +
      `    // 这行注释不该干扰抽取\n    value: serde_json::json!(DEFAULT_Y),\n},`,
    seedBlock("monitor_channels", "string", '"in_app"'),
    seedBlock("reco_trend_enabled", "boolean", "true"),
    seedBlock("value_dcf_discount_rate", "number", "DEFAULT_DCF_PCT"),
  ];
  const CORE_CONSTS = [
    "pub(crate) const DEFAULT_X: u32 = 8;",
    "pub(crate) const DEFAULT_Y: f64 = 0.3;",
    "pub(crate) const DEFAULT_DCF_PCT: f64 = axagent_fake_pkg::fake_mod::DISCOUNT_RATE * 100.0;",
  ].join("\n");
  const CORE_PANEL = [
    '  b("max_concurrent", 8, "x", "number");',
    '  b("agent_temperature", 0.3, "t", "number");',
    '  b("monitor_channels", "in_app", "c", "string");',
    '  b("reco_trend_enabled", true, "b", "boolean");',
    '  b("value_dcf_discount_rate", 7.7, "d", "number");',
  ];
  /** 注入的跨 crate 依赖文件：`0.077 * 100.0` 在 IEEE754 下是 7.700000000000001，
   *  正是 V1 容差要放行、而严格 `===` 会误判的那一类。 */
  const FAKE_DEP_FILES = {
    "src-tauri/crates/fake-pkg/src/fake_mod.rs": "pub const DISCOUNT_RATE: f64 = 0.077;",
  };
  const readFake = (rel) => {
    if (FAKE_DEP_FILES[rel]) {
      return FAKE_DEP_FILES[rel];
    }
    throw new Error(`selftest 未登记的依赖文件：${rel}`);
  };

  const CASES = [
    {
      title: "正样本：同文件 const / 字符串 / 布尔 / 跨 crate 派生式四类值全等 ⇒ 必须无错",
      seed: seedSrc(CORE_SEED, CORE_CONSTS),
      panel: panelSrc(CORE_PANEL),
      readDepFile: readFake,
      expectParse: 0,
      expectErr: 0,
    },
    {
      title: "负样本 A：面板值漂移（8→7）⇒ 必须报等式错",
      seed: seedSrc(CORE_SEED, CORE_CONSTS),
      panel: panelSrc(CORE_PANEL.map((l) => l.replace('b("max_concurrent", 8,', 'b("max_concurrent", 7,'))),
      readDepFile: readFake,
      expectParse: 0,
      expectErr: 1,
    },
    {
      title: "负样本 B：面板声明了种子没有的键 ⇒ 必须报键缺席（两处可设）",
      seed: seedSrc(CORE_SEED, CORE_CONSTS),
      panel: panelSrc(['  b("ghost_param", 1, "g", "number");', ...CORE_PANEL]),
      readDepFile: readFake,
      expectParse: 0,
      expectErr: 1,
    },
    {
      title: "负样本 C：类型漂移（面板 string / 种子 number）⇒ 必须报类型不等",
      seed: seedSrc(CORE_SEED, CORE_CONSTS),
      panel: panelSrc(CORE_PANEL.map((l) => l.replace('b("max_concurrent", 8, "x", "number")', 'b("max_concurrent", 8, "x", "string")'))),
      readDepFile: readFake,
      expectParse: 0,
      expectErr: 1,
    },
    {
      title: "负样本 D：种子常量引用了不存在的跨 crate 常量 ⇒ 必须报解析失败，不得当成 0",
      seed: seedSrc(CORE_SEED, CORE_CONSTS.replace("axagent_fake_pkg::fake_mod::DISCOUNT_RATE", "axagent_fake_pkg::fake_mod::NOT_A_REAL_CONST")),
      panel: panelSrc(CORE_PANEL),
      readDepFile: readFake,
      expectParse: 1,
      expectErr: 0,
    },
    {
      title: "边界样本 E：跨 crate 派生式被改成手抄字面量（7.7）⇒ 等式仍成立 ⇒ 本门判绿（自证边界：本门只保证「两处相等」，是不是派生自真相源由 check-valuation-defaults-parity.mjs 守）",
      seed: seedSrc(CORE_SEED, CORE_CONSTS.replace("axagent_fake_pkg::fake_mod::DISCOUNT_RATE * 100.0", "7.7")),
      panel: panelSrc(CORE_PANEL),
      readDepFile: readFake,
      expectParse: 0,
      expectErr: 0,
    },
    {
      title: "负样本 F：面板函数改名 ⇒ 必须报判据没电（空集不等于通过）",
      seed: seedSrc(CORE_SEED, CORE_CONSTS),
      panel: panelSrc(CORE_PANEL).replace("getDefaultVariables", "getOtherVariables"),
      readDepFile: readFake,
      expectParse: 1,
      expectErr: 0,
    },
    {
      title: "负样本 G：面板多出一种本门读不到的 `b(` 写法 ⇒ 必须报静默漏检（调用数 ≠ 解析条数）",
      seed: seedSrc(CORE_SEED, CORE_CONSTS),
      panel: panelSrc(['  b(nameOfThing, 8, "x", "number");', ...CORE_PANEL]),
      readDepFile: readFake,
      expectParse: 1,
      expectErr: 0,
    },
    {
      title: "负样本 H：种子块写法变了（value 不再走 json!）⇒ 必须报块形态未识别，且该键随之判成键缺席",
      seed: seedSrc(CORE_SEED, CORE_CONSTS).replace(
        'value: serde_json::json!(DEFAULT_Y),',
        'value: VariableValue::number(DEFAULT_Y),',
      ),
      panel: panelSrc(CORE_PANEL),
      readDepFile: readFake,
      expectParse: 1,
      expectErr: 1,
    },
  ];

  let failed = 0;
  console.log("股票分析变量默认值门禁 — 自检（正负对照）\n");
  for (const c of CASES) {
    const r = evaluate({ seedSrc: c.seed, panelSrc: c.panel, readDepFile: c.readDepFile });
    const ok =
      r.parseErrors.length === c.expectParse &&
      r.errors.length >= c.expectErr &&
      (c.expectErr === 0 ? r.errors.length === 0 : r.errors.length > 0);
    if (!ok) {
      failed++;
      // 失败时把逐条消息打出来：自检的「区分力不足」必须能定位到具体判据，否则只能靠猜。
      for (const e of [...r.parseErrors, ...r.errors]) {
        console.log(`        · ${e}`);
      }
    }
    console.log(
      `  ${ok ? "✓" : "✗"} ${c.title}\n` +
        `      解析错=${r.parseErrors.length}（期望 ${c.expectParse}）｜` +
        ` 等式错=${r.errors.length}（期望 ≥${c.expectErr}）`,
    );
  }
  console.log(
    failed === 0
      ? `\n✅ 自检通过（${CASES.length}/${CASES.length}）：门禁对所列全部失效形态均有区分力。`
      : `\n❌ 自检失败（${failed}/${CASES.length}）：门禁区分力不足，其「通过」不可信。`,
  );
  process.exit(failed === 0 ? 0 : 2);
}

if (process.argv.includes("--selftest")) {
  selftest();
}

// ── 主流程 ──
const readOrDie = (rel) => {
  try {
    return readFileSync(path.join(ROOT, rel), "utf8");
  } catch {
    return "";
  }
};

const { rows, parseErrors, errors } = evaluate({
  seedSrc: readOrDie(SEED_VARS),
  panelSrc: readOrDie(PANEL),
});

if (process.argv.includes("--dump")) {
  console.log("面板 b() 兜底值 ↔ 种子声明值 逐条读数");
  for (const r of rows) {
    console.log(
      `${r.ok ? "✓" : "✗"} ${String(r.name).padEnd(34)}面板=${String(r.panel).padEnd(12)}` +
        `种子=${r.seed} ⇒ ${r.seedValue}`,
    );
  }
  console.log(`\n共 ${rows.length} 条｜等式不符 ${errors.length}｜解析失败 ${parseErrors.length}\n`);
}

console.log("股票分析模板变量默认值等式门禁");
console.log(`权威源：${SEED_VARS}`);
console.log(`对照面：${PANEL}`);
console.log(
  `读数：面板 b() 声明 ${rows.length} 条 ｜ 等式不符 ${errors.length} 条 ｜ 解析失败 ${parseErrors.length} 条`,
);

if (parseErrors.length > 0) {
  console.log(`\n❌ 解析失败（${parseErrors.length} 项）—— 脚本自身失效，非「通过」：`);
  for (const e of parseErrors) {
    console.log(`  · ${e}`);
  }
  process.exit(2);
}

if (errors.length > 0) {
  console.log(`\n❌ 等式不符（${errors.length} 项）：`);
  for (const e of errors) {
    console.log(`  · ${e}`);
  }
  console.log(
    "\n   出厂默认值只能有一处权威：`seed_variables.rs` 是唯一来源，面板 `getDefaultVariables()`" +
      " 必须逐字段等值抄它（改种子同批改面板，并 bump 模板版本）。",
  );
  process.exit(1);
}

console.log("\n✅ 通过：面板兜底值与种子声明值逐字段一致（值 / 键 / 类型三面对账）。");
