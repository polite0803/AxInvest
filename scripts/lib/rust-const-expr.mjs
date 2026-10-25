// Rust `pub const` 定义式的**极简算术求值器**（`+ − * /`、括号、一元正负号）+ 不动点求解。
//
// ── 为什么从门禁里抽出来（2026-10-25）──
// 原先只有 `check-valuation-defaults-parity.mjs` 内置一份：真相源开始出现**派生常量**
// （`DISCOUNT_RATE = RISK_FREE_RATE + EQUITY_RISK_PREMIUM`）时，只认字面量的解析器会把
// 它判成「未解析到」。**故意的**：这种情况按「脚本自身失效(exit 2)」处理，而不是静默
// 跳过（判据 #7）。正确的扩展方向是让门禁**跟上写法**，而不是把常量改回字面量 ——
// 改回字面量等于重新引入可独立漂移的魔数，与门禁的目的相反。
//
// `check-stock-var-defaults-parity.mjs` 需要同一能力（种子变量表里有 6 个值取自同文件
// 常量、1 个取自跨 crate 常量），按 `AGENTS.md` 禁区 12「禁止重复定义」不再抄第二份，
// 而是共用本模块。
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
export function evalConstExpr(expr, resolved) {
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

/**
 * 迭代到不动点：一轮能解多少解多少，直到不再有进展（解决任意深度的派生链）。
 *
 * @param {Map<string, string>} defs 常量名 → 定义式文本
 * @param {Map<string, number>} [seed] 已知的常量值（跨文件 / 跨 crate 已解析出来的那些）
 * @returns {Map<string, number>} 能求出值的那部分；解不出的名字**不出现在结果里**，
 *          由调用方决定它是「暂不可解析」还是「判据没电」—— 本模块不替调用方把它当成 0。
 */
export function resolveConstsToFixedPoint(defs, seed = new Map()) {
  const values = new Map(seed);
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
  return values;
}
