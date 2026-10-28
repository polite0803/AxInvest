// SPDX-License-Identifier: AGPL-3.0-only
/**
 * Rust 源码「生产面」定位器 —— 全仓单一权威。
 *
 * 为什么抽成 lib：`check-port-axiom-wiring.mjs` 与 `check-contracts.mjs` 的 [H] 段问的是
 * 同一件事 —— 「这段代码算不算生产路径」。前者要行号可指的净化文本，后者要判断命中是否
 * 落在 `#[cfg(test)]` 块里。两边各写一份剥离器会让判据口径分叉（禁区 12「禁止重复定义」），
 * 这里收一份，两个门都 import 它。
 *
 * ⚠ 负控仍住在 `node scripts/check-port-axiom-wiring.mjs --selftest` —— 它那几条用例
 * （嵌套 fn 归属、测试模块起点/闭合）直接跑本文件的两个函数。改本文件后必须复跑那条门。
 */

/**
 * 把**注释与字符串字面量**替换成等长空白（换行原样保留），使行号与原文一一对应。
 *
 * 必须连字符串一起剥：Rust 里 `"/*"`、`"//"` 之类很常见（路径 glob / 正则 / URL），
 * 只剥注释会让状态机在字符串里读到 `/*` 而进入块注释态，**吞掉后面成百行代码**
 * —— 那是漏扫（致命）。第一版就是这么把 `seed_stock_analysis.rs` 的写入点整个吞掉的。
 *
 * 剥字符串则要正确处理 `'a` lifetime / `'x'` char / `r#"..."#` 原始字符串，否则
 * 同样会吞代码。这里：
 * - char 字面量按**码点长度**判定（`'中'` 是 3 个 UTF-16 码元），判不出来就当 lifetime
 *   —— 当 lifetime 是安全侧（不吞代码）；
 * - 原始字符串 `r"…"` / `r#"…"#` / `br#"…"#` 按 `"` + 同数量 `#` 找闭合。
 *
 * 等长性由调用点的行数守恒断言保证。
 */
export function sanitize(text) {
  let out = "";
  let i = 0;
  const n = text.length;
  let state = "code";
  const skip = (k) => {
    for (let t = 0; t < k && i < n; t++) {
      out += text[i] === "\n" ? "\n" : " ";
      i++;
    }
  };
  while (i < n) {
    const c = text[i];
    const c2 = text[i + 1];
    if (state === "code") {
      const raw = text.slice(i, i + 4).match(/^(?:b?r)(#*)"/);
      if (raw) {
        const closer = '"' + raw[1];
        const end = text.indexOf(closer, i + raw[0].length);
        const endIdx = end < 0 ? n : end + closer.length;
        skip(endIdx - i);
        continue;
      }
      if (c === "/" && c2 === "/") {
        state = "line";
        skip(2);
        continue;
      }
      if (c === "/" && c2 === "*") {
        state = "block";
        skip(2);
        continue;
      }
      if (c === '"') {
        state = "str";
        skip(1);
        continue;
      }
      if (c === "'") {
        let len = 0;
        if (text[i + 1] === "\\") {
          const m = text.slice(i + 1).match(/^\\(?:u\{[0-9a-fA-F_]{1,6}\}|x[0-9a-fA-F]{2}|.)/);
          if (m && text[i + 1 + m[0].length] === "'") len = m[0].length + 2;
        } else {
          const cp = text.codePointAt(i + 1);
          const cl = cp === undefined ? 0 : cp > 0xffff ? 2 : 1;
          if (cl && text[i + 1 + cl] === "'") len = cl + 2;
        }
        if (len) {
          skip(len);
          continue;
        }
        out += c;
        i++;
        continue;
      }
      out += c;
      i++;
      continue;
    }
    if (state === "line") {
      out += c === "\n" ? "\n" : " ";
      if (c === "\n") state = "code";
      i++;
      continue;
    }
    if (state === "block") {
      if (c === "*" && c2 === "/") {
        state = "code";
        skip(2);
        continue;
      }
      out += c === "\n" ? "\n" : " ";
      i++;
      continue;
    }
    // str
    if (c === "\\") {
      skip(2);
      continue;
    }
    if (c === '"') {
      state = "code";
      skip(1);
      continue;
    }
    out += c === "\n" ? "\n" : " ";
    i++;
  }
  return out;
}

/**
 * `#[cfg(test)]` 模块的行区间。测试夹具构造 `ActiveModel` 不是生产写路径 ——
 * 它只在 `cargo test` 的临时库里落行，没有用户数据可污染；把它当写入点报红
 * 只会逼人往测试里塞门禁标记（假接线）。生产写路径全部在 test 模块之外。
 *
 * ⚠ 只认**紧邻** `mod X {` 的属性（同一行，或去掉空行/其它属性行后的下一行）。
 * 属性贴在 `use` / `fn` / `impl` / `static` 上时**必须跳过这个属性**，不能往后找「下一个 mod」——
 * 那样会把那个 mod 的整段区间当成测试面，**静默豁免其间成百行生产代码**。
 * 现网读数（2026-10-28 实测，全仓 1018 处 `#[cfg(test)]`）：**67 处不紧贴 `mod X {`**，
 * 如 `src-tauri/src/commands/conversations/mod.rs:11` 的 `#[cfg(test)] pub use …`。
 * 按「往后找」的旧写法出数，实测在 `src-tauri/crates/rt-workflow/src/work_engine/engine/mod.rs`
 * 圈出 38–7365、在 `src-tauri/src/commands/conversations/mod.rs` 圈出 11–1800 ——
 * [H] 段会因此把 11 处生产丢弃一起藏掉，棘轮从 259 假降到 235（等于放松 24 格）。
 */
export function testModuleRanges(lines) {
  const ranges = [];
  const isAttrOnly = (l) => /^\s*#\[/.test(l) && !/\bmod\s/.test(l);
  for (let i = 0; i < lines.length; i++) {
    if (!/^\s*#\[cfg\(test\)\]/.test(lines[i])) continue;
    let j = i;
    if (!/\bmod\s+[A-Za-z_][A-Za-z0-9_]*\s*\{/.test(lines[j])) {
      j += 1;
      while (j < lines.length && (/^\s*$/.test(lines[j]) || isAttrOnly(lines[j]))) j += 1;
      // 下一行不是 `mod X {` ⇒ 这属性不属于模块，本属性不产生区间
      if (!/^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+[A-Za-z_][A-Za-z0-9_]*\s*\{/.test(lines[j] ?? "")) continue;
    }
    ranges.push([i, matchBraceEnd(lines, j)]);
    i = j;
  }
  return ranges;
}

/** 从 `startLine` 起按花括号配平，返回闭合行；找不到则返回最后一行。 */
export function matchBraceEnd(lines, startLine) {
  let depth = 0;
  let opened = false;
  for (let i = startLine; i < lines.length; i++) {
    for (const ch of lines[i]) {
      if (ch === "{") {
        depth++;
        opened = true;
      } else if (ch === "}") {
        depth--;
      }
    }
    if (opened && depth <= 0) return i;
  }
  return lines.length - 1;
}
