#!/usr/bin/env node
/**
 * 估值缺省参数 **等式门禁**。
 *
 * ## 为什么需要它
 *
 * 「估值缺省参数」（永续增长率 / 折现率 / 缺省增长率 / 增长率上下界 / 预测年数 /
 * 债券收益率基准）横跨 3 个技术栈、共 5 处载体：
 *
 * | 载体 | 位置 | 机制 |
 * |---|---|---|
 * | **单一真相源** | `src-tauri/crates/astock-data/src/mcp_tools.rs` | `pub const` |
 * | 设置页 DTO 缺省 | `src-tauri/src/commands/stock_analysis.rs` | `ValuationParams::default()` **派生** |
 * | 模板变量默认值 | `.../stock_analysis_setup/seed_variables.rs` | `DEFAULT_DCF_*_PCT` **派生**×100 |
 * | 决策层估值配置 | `src-tauri/crates/analysis-engine/src/decision.rs` | `ValueConfig::{default_dcf_*,Default}` **派生**×100（**2026-09-23 新增为第 5 处**） |
 * | 前端兜底显示 | `src/components/settings/StockAnalysisConfigPanel.tsx` | **手抄**（引用不到 Rust） |
 *
 * ### 第 5 处（`decision.rs`）的由来 —— 它曾是一个「无消费方」的漂移地雷
 *
 * 该结构此前手抄 `12.0 / 4.0 / 8.5` 且**全仓无消费方**（只有定义 + 注释引用）。
 * 而 `seed_stock_analysis.rs` 的 v32 变更日志记录了它的历史角色：2026-09-12 那次
 * A 股校准「只改到了**未被消费的** `decision::ValueConfig`，没改到实际执行的常量」。
 * **无消费方的副本最危险** —— 改它不生效、也无人报警。故本门禁把它纳入，并要求派生。
 *
 * ## 常量表达式的解析（2026-09-23 补）
 *
 * 真相源开始出现**两跳及以上的常量定义**（如某常量由两个具名分量之和/比派生）。
 * 本脚本内置一个**不执行被测代码**的极简算术求值器（`+ − * /`、括号、一元正负号、
 * 不动点迭代解依赖链），遇到引用了**不存在常量**的表达式会按「解析失败」报错
 * （exit 2）而不是把它当成 0 —— 后者会让一个坏表达式静默变成「值漂移」。
 *
 * ⚠️ 真实代码里 `DISCOUNT_RATE` **刻意写成字面量 + 编译期断言**而非表达式：
 * `0.025 + 0.06` 在 IEEE754 下为 `0.08499999999999999`，与本门禁的 `===` 比对不符。
 * 故「表达式支持」是为写法演进留的能力，不是鼓励把利率改成浮点求和。
 *
 * 2026-09-22 的 `AUDIT-300642-run-variance-2026-09-22.md` §9.6 证明这几份曾**各自停在
 * 不同校准批次**上 —— A 股校准（折现率 10→8.5、永续 3→4、缺省增长 8→12、
 * 下界 +2→−30）只落到其中一部分，另一些留在旧值。后果是**同一个参数在不同链上取不同值**：
 * 前端直调（WhatIf / 试算）经 `inject_valuation_config_for_tool` 注入 `ValuationParams`
 * 取到一套，工作流节点经 `input_mapping` 的扁平参数取到另一套。
 *
 * 主 crate / seed / 决策层现已改为**代码级派生**（`m::` / `axagent_astock_data::mcp_tools::*`），
 * 前端引用不到 Rust ⇒ 由本脚本守住等式。**新增第 6 处载体时请一并加入本门禁。**
 *
 * ## 判定规则（全部必须成立）
 *
 * 1. 前端 `DEFAULT_VALUATION_PARAMS` 的 7 个字段 与 真相源常量 **逐字段数值相等**；
 * 2. 主 crate `ValuationParams::default()` 的字段**全部以 `m::` 引用**（不得手抄字面量）；
 * 3. seed 的三个 `DEFAULT_DCF_*_PCT` **全部以 `axagent_astock_data::mcp_tools::*` 派生**。
 *
 * ⚠️ 任一项**解析不到**（0 命中）同样失败 —— 正则抓不到 ≠ 没问题，可能是写法变了。
 * 静默跳过正是本门禁要防的失效模式（判据 #7：`0 命中 ≠ 没问题`）。
 *
 * ## 用法
 *
 *   node scripts/check-valuation-defaults-parity.mjs             # 正式门禁
 *   node scripts/check-valuation-defaults-parity.mjs --selftest  # 正负对照（不读工作区）
 *
 * 退出码：0 通过 ｜ 1 等式不符 / 派生缺失 ｜ 2 解析失败（脚本自身失效，写法变了）
 */
import fs from "node:fs";
import path from "node:path";
import process from "node:process";

const ROOT = path.resolve(import.meta.dirname, "..");

const SRC_OF_TRUTH = "src-tauri/crates/astock-data/src/mcp_tools.rs";
const MAIN_CRATE = "src-tauri/src/commands/stock_analysis.rs";
const SEED_VARS = "src-tauri/src/commands/stock_analysis_setup/seed_variables.rs";
const FRONTEND = "src/components/settings/StockAnalysisConfigPanel.tsx";
const DECISION_CRATE = "src-tauri/crates/analysis-engine/src/decision.rs";

/**
 * `decision.rs` 的三个派生函数 ↔ 真相源常量。
 *
 * ⚠ 为什么必须单列（2026-09-23）：本文件曾是一个「**无消费方**」的独立副本
 * （`12.0 / 4.0 / 8.5` 裸字面量），历史上正是「假修复」的载体 ——
 * v32 变更日志记录「只改到了**未被消费的** `decision::ValueConfig`」。
 * 2026-09-23 已改为派生，但**只在文档头声称它是第 5 处 ≠ 它真的被校验**：
 * 实测本文件当时仍只 `evaluate({truthSrc, mainSrc, seedSrc, frontSrc})` 四路输入
 * ⇒ `decision.rs` 一个字都没被看。本表 + `checkDecisionCrate` 就是把
 * 「声明与实现不一致」补上的那一步（否则门禁只在纸上多了一处）。
 */
const DECISION_FNS = [
  { fn: "default_dcf_growth", rust: "DEFAULT_GROWTH" },
  { fn: "default_dcf_perpetual", rust: "PERPETUAL_GROWTH" },
  { fn: "default_dcf_discount", rust: "DISCOUNT_RATE" },
];

/** 字段对照表：前端字段名 ↔ 真相源常量名 */
const MAP = [
  { front: "perpetualGrowth", rust: "PERPETUAL_GROWTH" },
  { front: "discountRate", rust: "DISCOUNT_RATE" },
  { front: "defaultGrowth", rust: "DEFAULT_GROWTH" },
  { front: "minGrowth", rust: "MIN_GROWTH" },
  { front: "maxGrowth", rust: "MAX_GROWTH" },
  { front: "forecastYears", rust: "FORECAST_YEARS" },
  { front: "bondYield", rust: "DEFAULT_BOND_YIELD" },
];

const SEED_CONSTS = [
  { name: "DEFAULT_DCF_GROWTH_RATE_PCT", rust: "DEFAULT_GROWTH" },
  { name: "DEFAULT_DCF_PERPETUAL_RATE_PCT", rust: "PERPETUAL_GROWTH" },
  { name: "DEFAULT_DCF_DISCOUNT_RATE_PCT", rust: "DISCOUNT_RATE" },
];

/** 解析失败清单（exit 2）与等式失败清单（exit 1）分开，便于 CI 区分「脚本坏了」与「值错了」 */
const parseErrors = [];
const errors = [];

// ── 解析器（纯函数，供 selftest 直接喂合成样本） ──

// ── 极简算术求值器（`+ − * /`、括号、一元正负号）──
//
// 为什么需要它（2026-09-23）：真相源开始出现**派生常量**（`DISCOUNT_RATE =
// RISK_FREE_RATE + EQUITY_RISK_PREMIUM`）—— 原解析器只认 `<数字>` 字面量，遇到派生式
// 会报「未解析到」。**故意的**：它把这种情况判为「脚本自身失效(exit 2)」而不是静默
// 跳过（判据 #7）。正确的扩展方向是让门禁**跟上写法**，而不是把常量改回字面量
// —— 改回字面量等于重新引入可独立漂移的魔数，与本门禁的目的相反。
//
// 刻意**不使用** `new Function` / `eval`：这是 CI 门禁，不执行被测文本里的任何代码。
function tokenizeArith(s) {
  const t = [];
  let i = 0;
  while (i < s.length) {
    const c = s[i];
    if (/\s/.test(c)) {
      i++;
      continue;
    }
    if (/[\d.]/.test(c)) {
      let j = i;
      while (j < s.length && /[\d.]/.test(s[j])) {
        j++;
      }
      t.push({ k: "num", v: Number(s.slice(i, j)) });
      i = j;
      continue;
    }
    if ("+-*/()".includes(c)) {
      t.push({ k: c });
      i++;
      continue;
    }
    return null; // 未知字符（未解析的标识符、或 `f64::` 之类）
  }
  return t;
}

function parseExprArith(t, pos) {
  let r = parseTermArith(t, pos);
  if (!r) {
    return null;
  }
  let [v, p] = r;
  while (p < t.length && (t[p].k === "+" || t[p].k === "-")) {
    const op = t[p].k;
    const rr = parseTermArith(t, ++p);
    if (!rr) {
      return null;
    }
    v = op === "+" ? v + rr[0] : v - rr[0];
    p = rr[1];
  }
  return [v, p];
}

function parseTermArith(t, pos) {
  let r = parseUnaryArith(t, pos);
  if (!r) {
    return null;
  }
  let [v, p] = r;
  while (p < t.length && (t[p].k === "*" || t[p].k === "/")) {
    const op = t[p].k;
    const rr = parseUnaryArith(t, ++p);
    if (!rr) {
      return null;
    }
    v = op === "*" ? v * rr[0] : v / rr[0];
    p = rr[1];
  }
  return [v, p];
}

function parseUnaryArith(t, pos) {
  if (pos < t.length && (t[pos].k === "-" || t[pos].k === "+")) {
    const r = parseUnaryArith(t, pos + 1);
    if (!r) {
      return null;
    }
    return [t[pos].k === "-" ? -r[0] : r[0], r[1]];
  }
  return parseAtomArith(t, pos);
}

function parseAtomArith(t, pos) {
  if (pos >= t.length) {
    return null;
  }
  if (t[pos].k === "num") {
    return [t[pos].v, pos + 1];
  }
  if (t[pos].k === "(") {
    const r = parseExprArith(t, pos + 1);
    if (!r) {
      return null;
    }
    const [v, p] = r;
    if (p >= t.length || t[p].k !== ")") {
      return null;
    }
    return [v, p + 1];
  }
  return null;
}

/**
 * 把常量定义式求值为数字。`resolved` 是已解析常量的 Map。
 * 未知标识符 ⇒ 返回 null（**不**当成 0），使调用方按「暂不可解析」处理并等下一轮。
 */
function evalConstExpr(expr, resolved) {
  const substituted = expr
    .replace(/(\d)_(?:f64|f32|i32|i64|usize|u32)\b/g, "$1")
    .replace(/\b([A-Za-z_][A-Za-z0-9_]*)\b/g, (name) =>
      resolved.has(name) ? `(${resolved.get(name)})` : `\u0000${name}`);
  const t = tokenizeArith(substituted);
  if (!t || t.length === 0) {
    return null;
  }
  const r = parseExprArith(t, 0);
  if (!r || r[1] !== t.length) {
    return null;
  }
  return Number.isFinite(r[0]) ? r[0] : null;
}

/** 真值源：抓 `pub const <NAME>: f64|i32 = <数字 或 常量表达式>;` 并按依赖顺序求值 */
export function parseTruth(src) {
  const defs = new Map();
  for (const m of src.matchAll(
    /^pub const ([A-Z][A-Z0-9_]*)\s*:\s*(?:f64|i32)\s*=\s*([^;]+);/gm,
  )) {
    defs.set(m[1], m[2].trim());
  }
  // 迭代到不动点：一轮能解多少解多少，直到不再有进展（解决任意深度的派生链）。
  const values = new Map();
  for (let pass = 0; pass <= defs.size; pass++) {
    let progress = false;
    for (const [name, expr] of defs) {
      if (values.has(name)) {
        continue;
      }
      const v = evalConstExpr(expr, values);
      if (v !== null) {
        values.set(name, v);
        progress = true;
      }
    }
    if (!progress) {
      break;
    }
  }

  const out = {};
  const miss = [];
  for (const { rust } of MAP) {
    if (values.has(rust)) {
      out[rust] = values.get(rust);
    } else {
      miss.push(
        `${rust}（须为 \`pub const ${rust}: f64|i32 = <数字|常量表达式>;\`，` +
          `且表达式中引用的常量本身也可解析）`,
      );
    }
  }
  return { values: out, missing: miss };
}

/** 前端：抓 `DEFAULT_VALUATION_PARAMS` 字面量块内的 7 个字段 */
export function parseFrontend(src) {
  const block = src.match(
    /const DEFAULT_VALUATION_PARAMS:\s*ValuationParamsConfig\s*=\s*\{([\s\S]*?)\};/,
  );
  if (!block) {
    return { values: {}, missing: ["<整个字面量块>"] };
  }
  const out = {};
  const miss = [];
  for (const { front } of MAP) {
    const m = block[1].match(new RegExp(`\\b${front}\\s*:\\s*(-?[\\d.]+)\\s*,`));
    if (!m) {
      miss.push(front);
    } else {
      out[front] = Number(m[1]);
    }
  }
  return { values: out, missing: miss };
}

/** 主 crate：`impl Default for ValuationParams` 内每个字段必须派生（`m::`） */
export function checkMainCrate(src) {
  const block = src.match(/impl Default for ValuationParams\s*\{([\s\S]*?)\n\}/);
  if (!block) {
    return { missing: ["<impl Default for ValuationParams>"], bad: [] };
  }
  // 剔除注释，避免注释里的数字被误判；只保留 `字段: 表达式,` 形态。
  const body = block[1]
    .split("\n")
    .map((l) => l.replace(/\/\/.*$/, ""))
    .filter((l) => l.trim().length > 0);
  const assignments = body
    .map((l) => l.match(/^\s*([a-z_][a-z0-9_]*)\s*:\s*(.+?)\s*,\s*$/))
    .filter(Boolean);
  if (assignments.length === 0) {
    return { missing: ["<字段赋值（写法变了？）>"], bad: [] };
  }
  const bad = assignments
    .filter(([, , expr]) => !/^\s*m::[A-Z_]+\s*$/.test(expr))
    .map(([, field, expr]) => ({ field, expr: expr.trim() }));
  return { missing: [], bad };
}

/** seed：三个百分数常量必须派生自真相源 ×100 */
export function checkSeed(src) {
  const missing = [];
  for (const { name } of SEED_CONSTS) {
    const re = new RegExp(
      `${name}\\s*:\\s*f64\\s*=\\s*axagent_astock_data::mcp_tools::[A-Z_]+\\s*\\*\\s*100\\.0\\s*;`,
    );
    if (!re.test(src)) {
      missing.push(name);
    }
  }
  return { missing };
}

/**
 * 校验 `decision.rs` 的三个 `default_dcf_*()` 是否**派生**自真相源。
 *
 * ⚠ 断言必须锚在**派生式**上，不能只断言「函数存在」—— 旧实现（裸字面量
 * `12.0 / 4.0 / 8.5`）同样满足「函数存在」⇒ 只判存在性的门禁对这个文件毫无区分力。
 */
export function checkDecisionCrate(src) {
  const missing = [];
  const bad = [];
  for (const { fn, rust } of DECISION_FNS) {
    const body = new RegExp(`fn\\s+${fn}\\s*\\(\\s*\\)\\s*->\\s*f64\\s*\\{([\\s\\S]*?)\\}`).exec(src);
    if (!body) {
      missing.push(fn);
      continue;
    }
    // 函数体必须是 `<...mcp_tools>::<CONST> * 100.0`（允许 `use ... as m` 的别名写法）
    const derived = new RegExp(
      `(?:axagent_astock_data::mcp_tools|\\bm)::${rust}\\s*\\*\\s*100\\.0`,
    );
    if (!derived.test(body[1])) {
      bad.push({ field: fn, expr: body[1].trim().replace(/\s+/g, " ") });
    }
  }
  return { missing, bad };
}

/**
 * 汇总判定：5 处载体的一致性。
 *
 * （2026-09-23：由 **4 处扩到 5 处** —— 加入 `decision.rs`。上一版**只在文档头**
 *   写了 5 处，而本函数仍只收 4 路输入 ⇒ `decision.rs` 一个字都没被看，
 *   属典型的「声明与实现不一致」。补法 = 让它真的进输入面。）
 *
 * @returns {{rows: Array, parseErrors: string[], errors: string[]}}
 */
export function evaluate({ truthSrc, mainSrc, seedSrc, frontSrc, decisionSrc = "" }) {
  const pErr = [];
  const err = [];

  // 第 5 处：decision.rs（无消费方地雷，见 `DECISION_FNS` 上方注释）
  const decision = checkDecisionCrate(decisionSrc);
  for (const n of decision.missing) {
    pErr.push(`[decision] 未解析到 \`fn ${n}() -> f64 { ... }\``);
  }
  for (const { field, expr } of decision.bad) {
    err.push(
      `[decision] \`${field}()\` 未派生自真相源：\`${expr}\` —— ` +
        `须写成 \`axagent_astock_data::mcp_tools::<常量> * 100.0\`，禁止手抄字面量`,
    );
  }

  const truth = parseTruth(truthSrc);
  for (const n of truth.missing) {
    pErr.push(`[真相源] 未解析到 \`pub const ${n}\`（须为 \`pub const ${n}: f64|i32 = <数字>;\`）`);
  }

  const front = parseFrontend(frontSrc);
  for (const n of front.missing) {
    pErr.push(`[前端] 未解析到字段 \`${n}\``);
  }

  const main = checkMainCrate(mainSrc);
  for (const n of main.missing) {
    pErr.push(`[主 crate] 未定位 ${n}`);
  }
  for (const { field, expr } of main.bad) {
    err.push(
      `[主 crate] \`ValuationParams::default()\` 的 \`${field}\` 未派生自真相源：` +
        `\`${expr}\` —— 须写成 \`m::<常量名>\`，禁止手抄字面量`,
    );
  }

  const seed = checkSeed(seedSrc);
  for (const n of seed.missing) {
    err.push(
      `[seed] \`${n}\` 未派生自真相源 —— 须写成 ` +
        `\`= axagent_astock_data::mcp_tools::<常量> * 100.0;\``,
    );
  }

  const rows = [];
  for (const { front: key, rust } of MAP) {
    const a = truth.values[rust];
    const b = front.values[key];
    const ok = a !== undefined && b !== undefined && a === b;
    rows.push({ key, rust, truth: a, front: b, ok });
    if (!ok && a !== undefined && b !== undefined) {
      err.push(`字段 \`${rust}\` / \`${key}\` 不相等：真相源=${a}，前端=${b}`);
    }
  }

  return { rows, parseErrors: pErr, errors: err };
}

// ── 自检：正负对照（不读工作区文件，纯合成样本） ──
function selftest() {
  const TRUTH = [
    "/// doc",
    "pub const PERPETUAL_GROWTH: f64 = 0.04;",
    "pub const DISCOUNT_RATE: f64 = 0.085;",
    "pub const DEFAULT_GROWTH: f64 = 0.12;",
    "pub const MIN_GROWTH: f64 = -0.30;",
    "pub const MAX_GROWTH: f64 = 0.30;",
    "pub const FORECAST_YEARS: i32 = 5;",
    "pub const DEFAULT_BOND_YIELD: f64 = 4.4;",
  ].join("\n");

  const FRONT_OK = [
    "const DEFAULT_VALUATION_PARAMS: ValuationParamsConfig = {",
    "  perpetualGrowth: 0.04,",
    "  discountRate: 0.085,",
    "  defaultGrowth: 0.12,",
    "  minGrowth: -0.3,",
    "  maxGrowth: 0.3,",
    "  forecastYears: 5,",
    "  bondYield: 4.4,",
    "};",
  ].join("\n");

  const MAIN_OK = [
    "impl Default for ValuationParams {",
    "    fn default() -> Self {",
    "        use axagent_astock_data::mcp_tools as m;",
    "        Self {",
    "            perpetual_growth: m::PERPETUAL_GROWTH,",
    "            discount_rate: m::DISCOUNT_RATE,",
    "            default_growth: m::DEFAULT_GROWTH,",
    "            min_growth: m::MIN_GROWTH,",
    "            max_growth: m::MAX_GROWTH,",
    "            forecast_years: m::FORECAST_YEARS,",
    "            bond_yield: m::DEFAULT_BOND_YIELD,",
    "        }",
    "    }",
    "}",
  ].join("\n");

  const SEED_OK = [
    "pub(crate) const DEFAULT_DCF_GROWTH_RATE_PCT: f64 =",
    "    axagent_astock_data::mcp_tools::DEFAULT_GROWTH * 100.0;",
    "pub(crate) const DEFAULT_DCF_PERPETUAL_RATE_PCT: f64 =",
    "    axagent_astock_data::mcp_tools::PERPETUAL_GROWTH * 100.0;",
    "pub(crate) const DEFAULT_DCF_DISCOUNT_RATE_PCT: f64 =",
    "    axagent_astock_data::mcp_tools::DISCOUNT_RATE * 100.0;",
  ].join("\n");

  // 第 5 处：decision.rs 的三个 serde 默认函数（形态与真实源码一致，含换行）。
  const DECISION_OK = [
    "fn default_dcf_growth() -> f64 {",
    "    axagent_astock_data::mcp_tools::DEFAULT_GROWTH * 100.0",
    "}",
    "fn default_dcf_perpetual() -> f64 {",
    "    axagent_astock_data::mcp_tools::PERPETUAL_GROWTH * 100.0",
    "}",
    "fn default_dcf_discount() -> f64 {",
    "    axagent_astock_data::mcp_tools::DISCOUNT_RATE * 100.0",
    "}",
  ].join("\n");

  const cases = [
    {
      title: "正样本：4 处一致 ⇒ 必须无错",
      input: { truthSrc: TRUTH, mainSrc: MAIN_OK, seedSrc: SEED_OK, frontSrc: FRONT_OK },
      expectParse: 0,
      expectErr: 0,
    },
    {
      title: "负样本 A：前端值漂移（0.12→0.13）⇒ 必须报等式错",
      input: {
        truthSrc: TRUTH,
        mainSrc: MAIN_OK,
        seedSrc: SEED_OK,
        frontSrc: FRONT_OK.replace("defaultGrowth: 0.12,", "defaultGrowth: 0.13,"),
      },
      expectParse: 0,
      expectErr: 1,
    },
    {
      title: "负样本 B：主 crate 手抄字面量 ⇒ 必须报派生缺失",
      input: {
        truthSrc: TRUTH,
        mainSrc: MAIN_OK.replace("min_growth: m::MIN_GROWTH,", "min_growth: 0.02,"),
        seedSrc: SEED_OK,
        frontSrc: FRONT_OK,
      },
      expectParse: 0,
      expectErr: 1,
    },
    {
      title: "负样本 C：seed 未派生（手抄 12.0）⇒ 必须报派生缺失",
      input: {
        truthSrc: TRUTH,
        mainSrc: MAIN_OK,
        seedSrc: SEED_OK.replace(
          "axagent_astock_data::mcp_tools::DEFAULT_GROWTH * 100.0",
          "12.0",
        ),
        frontSrc: FRONT_OK,
      },
      expectParse: 0,
      expectErr: 1,
    },
    {
      title: "负样本 D：真相源写法变了（常量解析 0 命中）⇒ 必须报解析失败",
      input: {
        truthSrc: TRUTH.replace("pub const DEFAULT_GROWTH: f64 = 0.12;", "// 注释掉了"),
        mainSrc: MAIN_OK,
        seedSrc: SEED_OK,
        frontSrc: FRONT_OK,
      },
      expectParse: 1,
      expectErr: 0,
    },
    {
      title: "正样本 B：真相源含**两跳常量表达式**（DISCOUNT_RATE ← 千分基点数 ÷ 10000）⇒ 必须求值成 0.085 并比对通过",
      input: {
        truthSrc: TRUTH.replace(
          "pub const DISCOUNT_RATE: f64 = 0.085;",
          // 两跳：DISCOUNT_RATE 依赖 RATE_BP，RATE_BP 依赖字面量 ⇒ 覆盖「不动点迭代」。
          // ⚠️ 刻意**不**用 `0.025 + 0.06`：其 IEEE754 结果为 0.08499999999999999，
          //    与前端字面量 `0.085` **不严格相等** ⇒ 本门禁（`===`）会（正确地）报不等。
          //    真实代码正是为此不用表达式而用「字面量 + 编译期断言」，见 mcp_tools.rs。
          "pub const RATE_BP: f64 = 850.0;\n" + "pub const DISCOUNT_RATE: f64 = RATE_BP / 10000.0;",
        ),
        mainSrc: MAIN_OK,
        seedSrc: SEED_OK,
        frontSrc: FRONT_OK,
      },
      expectParse: 0,
      expectErr: 0,
    },
    {
      title: "负样本 E：常量表达式引用了不存在的常量 ⇒ 必须报解析失败，而非把它当成 0",
      input: {
        truthSrc: TRUTH.replace(
          "pub const DISCOUNT_RATE: f64 = 0.085;",
          "pub const DISCOUNT_RATE: f64 = NOT_A_REAL_CONST + 0.06;",
        ),
        mainSrc: MAIN_OK,
        seedSrc: SEED_OK,
        frontSrc: FRONT_OK,
      },
      expectParse: 1,
      expectErr: 0,
    },
    {
      title: "负样本 F：decision.rs（第 5 处）手抄字面量 12.0 ⇒ 必须报派生缺失",
      input: {
        truthSrc: TRUTH,
        mainSrc: MAIN_OK,
        seedSrc: SEED_OK,
        frontSrc: FRONT_OK,
        decisionSrc: DECISION_OK.replace(
          "axagent_astock_data::mcp_tools::DEFAULT_GROWTH * 100.0",
          "12.0",
        ),
      },
      expectParse: 0,
      expectErr: 1,
    },
    {
      title: "负样本 G：decision.rs 缺 default_dcf_perpetual() ⇒ 必须报解析失败",
      input: {
        truthSrc: TRUTH,
        mainSrc: MAIN_OK,
        seedSrc: SEED_OK,
        frontSrc: FRONT_OK,
        decisionSrc: DECISION_OK.replace(/fn default_dcf_perpetual[\s\S]*?\}/, ""),
      },
      expectParse: 1,
      expectErr: 0,
    },
  ];

  // 每个用例默认带上第 5 处（decision.rs）的**正样本**；
  // 需要检验第 5 处失效的用例自行覆盖 `decisionSrc`（展开顺序保证覆盖生效）。
  const withDecision = (input) => ({ decisionSrc: DECISION_OK, ...input });

  let failed = 0;
  console.log("估值缺省参数门禁 — 自检（正负对照）\n");
  for (const c of cases) {
    const r = evaluate(withDecision(c.input));
    const ok = r.parseErrors.length === c.expectParse && r.errors.length >= c.expectErr;
    if (!ok) {
      failed++;
    }
    console.log(
      `  ${ok ? "✓" : "✗"} ${c.title}\n` +
        `      解析错=${r.parseErrors.length}（期望 ${c.expectParse}）｜` +
        ` 等式错=${r.errors.length}（期望 ≥${c.expectErr}）`,
    );
  }
  console.log(
    failed === 0
      ? `\n✅ 自检通过（${cases.length}/${cases.length}）：门禁对所列全部失效形态均有区分力（含第 5 处 decision.rs 的正负样本）。`
      : `\n❌ 自检失败（${failed}/${cases.length}）：门禁区分力不足，其「通过」不可信。`,
  );
  process.exit(failed === 0 ? 0 : 2);
}

// ── 主流程 ──
if (process.argv.includes("--selftest")) {
  selftest();
}

const readOrDie = (rel) => {
  const p = path.join(ROOT, rel);
  if (!fs.existsSync(p)) {
    parseErrors.push(`文件不存在：${rel}`);
    return "";
  }
  return fs.readFileSync(p, "utf8");
};

const { rows, parseErrors: pe, errors: es } = evaluate({
  truthSrc: readOrDie(SRC_OF_TRUTH),
  mainSrc: readOrDie(MAIN_CRATE),
  seedSrc: readOrDie(SEED_VARS),
  frontSrc: readOrDie(FRONTEND),
  decisionSrc: readOrDie(DECISION_CRATE),
});
parseErrors.push(...pe);
errors.push(...es);

console.log("估值缺省参数等式门禁");
console.log(`真相源：${SRC_OF_TRUTH}`);
console.log("");
console.log("字段".padEnd(18) + "真相源常量".padEnd(22) + "值".padEnd(10) + "前端值");
for (const r of rows) {
  console.log(
    String(r.key).padEnd(16) +
      String(r.rust).padEnd(24) +
      String(r.truth).padEnd(10) +
      String(r.front) +
      (r.ok ? "  ✓" : "  ✗"),
  );
}

if (parseErrors.length > 0) {
  console.log(`\n❌ 解析失败（${parseErrors.length} 项）—— 脚本自身失效，非「通过」：`);
  for (const e of parseErrors) {
    console.log("  - " + e);
  }
  process.exit(2);
}
if (errors.length > 0) {
  console.log(`\n❌ 失败（${errors.length} 项）:`);
  for (const e of errors) {
    console.log("  - " + e);
  }
  process.exit(1);
}
console.log(
  `\n✅ 通过：5 处载体逐字段一致，且主 crate / seed / decision 均为代码级派生而非手抄。\n` +
    `   真相源：${SRC_OF_TRUTH}`,
);
