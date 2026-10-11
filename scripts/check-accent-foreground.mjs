#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
//
// 形态门：禁止把「压在 `--accent` 上的前景色」写死成白。
//
// 为什么需要（2026-10-09 实证）：`21th-dark` 档的 `--accent` 就是**纯白**
// （`src/index.css` 里 `oklch(100% 0 0)`），而工作区 8 处激活态按钮一律写
// `color: "white"` ⇒ 白底白字，用户看到的是两个空白块，且**没有任何报错**。
// antd 那一半同源：`colorTextLightSolid` 从未设置，默认 `#fff`，于是
// `21th-dark` 的主按钮（`primaryColor: "#ffffff"`）也白底白字。
//
// 正解是 `--accent-fg` / `primaryTextColor` 两个 token。本门只锁「写死白」这个形态，
// 不判对比度数值 —— 对比度要引入亮度计算，而形态违规占了实际发生面的 100%。
//
// 用法：
//   node scripts/check-accent-foreground.mjs            # 门禁
//   node scripts/check-accent-foreground.mjs --selftest  # 正负对照

import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const ROOT = process.cwd();
const TOKEN = "--accent-fg";

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (name === "node_modules" || name.startsWith(".")) { continue; }
    const st = statSync(p);
    if (st.isDirectory()) { walk(p, out); }
    else if (/\.tsx?$/.test(name)) { out.push(p); }
  }
  return out;
}

/** 剥掉注释，避免把注释里的示例当违规（也避免注释里的 `var(--accent)` 造成假阳）。 */
export function stripComments(src) {
  return src
    .replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, " "))
    .replace(/(^|[^:])\/\/[^\n]*/g, "$1");
}

const WHITE_RE = /^(white|#fff|#ffffff)$/i;

/**
 * 找到所有「`background` 取 `var(--accent)`」的位置，返回其所在对象字面量内
 * 被写死成白的 `color` 行。对象字面量按花括号配对取（折行不影响判定）。
 */
export function findViolations(text) {
  const hits = [];
  const bgRe = /background(?:Color)?\s*:\s*[^,;}\n]*var\(--accent\)/g;
  let m;
  while ((m = bgRe.exec(text)) !== null) {
    // 从匹配点向前找最近的 `{`，向后找配对的 `}` ⇒ 该 style 对象体的范围
    const open = text.lastIndexOf("{", m.index);
    if (open < 0) { continue; }
    let depth = 0;
    let close = -1;
    for (let i = open; i < text.length; i++) {
      if (text[i] === "{") { depth++; }
      else if (text[i] === "}") {
        depth--;
        if (depth === 0) { close = i; break; }
      }
    }
    if (close < 0) { continue; }
    const body = text.slice(open, close + 1);
    // 值段取到下一个 `,` 或 `}` 为止 ⇒ 三元两侧分支（`a ? "white" : "var(--muted)"`）都在段内
    const colorRe = /(?:^|[{,])\s*color\s*:\s*([^,}]+)/g;
    let c;
    while ((c = colorRe.exec(body)) !== null) {
      const lits = [...c[1].matchAll(/(["'])([^"']*)\1/g)].map((x) => x[2]);
      const white = lits.find((v) => WHITE_RE.test(v.trim()));
      if (white !== undefined) {
        // 行号按 `color` 本身定位，不按匹配起点（起点含前置 `,` / `{`，会少一行）
        const abs = open + body.indexOf("color", c.index);
        hits.push({ line: text.slice(0, text.lastIndexOf("\n", abs) + 1).split("\n").length, value: white });
      }
    }
  }
  return hits;
}

/** antd 侧：`colorPrimary` 偏浅的档位必须声明 `primaryTextColor`。 */
export function findAntdTonalGaps(themeText) {
  const missing = [];
  const blockRe = /"([a-z0-9-]+)":\s*\{([\s\S]*?)\n\s{2}\},/g;
  let m;
  while ((m = blockRe.exec(themeText)) !== null) {
    const [, name, body] = m;
    const pm = /primaryColor:\s*"([^"]+)"/.exec(body);
    if (!pm) { continue; }
    const hex = /^#([0-9a-f]{6})$/i.exec(pm[1].trim());
    if (!hex) { continue; } // oklch 形态的档位本门不判（当前无 hex 之外的浅主色档）
    const n = parseInt(hex[1], 16);
    const [r, g, b] = [(n >> 16) & 255, (n >> 8) & 255, n & 255].map((v) => v / 255);
    const lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    const hasFg = /primaryTextColor:/.test(body);
    if (lum > 0.55 && !hasFg) { missing.push({ name, primary: pm[1], lum: lum.toFixed(2) }); }
  }
  return missing;
}

function selftest() {
  const cases = [];
  const ok = (name, pass) => cases.push({ name, pass });

  ok("正控：同一对象内写死 white 被抓",
    findViolations(`x={<button style={{ background: "var(--accent)", color: "white" }} />}`).length === 1);
  ok("正控：三元里的激活分支被抓",
    findViolations(`style={{ background: a ? "var(--accent)" : "t", color: a ? "white" : "var(--muted)" }}`).length === 1);
  ok("正控：折行形态照样被抓",
    findViolations(`style={{\n  background: isActive ? "var(--accent)" : "transparent",\n  color: isActive ? "#ffffff" : "var(--muted)",\n}}`).length === 1);
  ok("负控：token 形态不报",
    findViolations(`style={{ background: "var(--accent)", color: "var(--accent-fg)" }}`).length === 0);
  ok("负控：accent 当文字色（无背景）不报",
    findViolations(`style={{ color: "var(--accent)" }}`).length === 0);
  ok("负控：注释里的写死白不报",
    findViolations(`// 此前是 color: "white" 配 background: "var(--accent)"\nstyle={{ background: "var(--accent)", color: "var(--accent-fg)" }}`).length === 0);
  ok("负控：非 accent 背景上的 white 不报（本门不管）",
    findViolations(`style={{ background: "#000", color: "white" }}`).length === 0);
  ok("antd：浅主色档缺 primaryTextColor 被抓",
    findAntdTonalGaps(`  "t-dark": {\n    primaryColor: "#ffffff",\n    borderRadiusBias: 0,\n  },`).length === 1);
  ok("antd：浅主色档已声明则不报",
    findAntdTonalGaps(`  "t-dark": {\n    primaryColor: "#ffffff",\n    primaryTextColor: "#0a0a0a",\n  },`).length === 0);
  ok("antd：深主色档不报",
    findAntdTonalGaps(`  "t-light": {\n    primaryColor: "#111111",\n  },`).length === 0);

  for (const c of cases) { console.log(`${c.pass ? "OK  " : "FAIL"} ${c.name}`); }
  const bad = cases.filter((c) => !c.pass).length;
  console.log(`\n自检 ${cases.length - bad}/${cases.length} 通过`);
  process.exit(bad ? 1 : 0);
}

if (process.argv.includes("--selftest")) { selftest(); }

const files = walk(join(ROOT, "src"));
let violations = 0;
for (const f of files) {
  const text = stripComments(readFileSync(f, "utf8"));
  for (const h of findViolations(text)) {
    console.log(`✖ ${relative(ROOT, f)}:${h.line} 压在 --accent 上的前景写死成 ${h.value}`);
    violations++;
  }
}

const css = readFileSync(join(ROOT, "src", "index.css"), "utf8");
if (!css.includes(`${TOKEN}:`)) {
  console.log(`✖ src/index.css 未声明 ${TOKEN} —— 消费端 var(${TOKEN}) 会静默退回继承色`);
  violations++;
}

const theme = readFileSync(join(ROOT, "src", "theme", "shadcnTheme.ts"), "utf8");
for (const g of findAntdTonalGaps(theme)) {
  console.log(`✖ src/theme/shadcnTheme.ts 档位 "${g.name}" 的 primaryColor=${g.primary}（亮度 ${g.lum}）缺 primaryTextColor ⇒ 主按钮白底白字`);
  violations++;
}

if (violations > 0) {
  console.log(`\n✖ ${violations} 处。正解：CSS 用 var(--accent-fg)，antd 用 primaryTextColor。`);
  process.exit(1);
}
console.log(`✅ 压在 --accent 上的前景全部走 token（扫 ${files.length} 个 ts/tsx + index.css + 13 档 antd 预设）`);
