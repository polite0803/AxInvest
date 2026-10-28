// SPDX-License-Identifier: AGPL-3.0-only
//
// 主题 token 单一权威判据（2026-10-28 收编 `--sa-*` 伪 token 时立，同日升级为通用门）
//
// 立门的直接证据：`--sa-red / --sa-green / --sa-amber / --sa-*-bg` 这一族在 index.css
// （含 git 历史）里**从未定义过**，却被 45 个文件写了 288 处 —— 其中 198 处不带兜底
// ⇒ 整条 CSS 声明失效（红/绿/琥珀语义色根本没画出来），其余 90 处恒吃兜底的 tailwind
// 色值、与主题无关。同屏「一个红」有两套来源，正是「换机/暗色才暴露的颜色不一」。
//
// 通用判据：**代码里出现的每个 var(--x) 名字，都必须能在定义面找到**
// （index.css / 其它 .css 的声明，或运行时 `setProperty("--x", …)` 的投影）。
// 三个函数都是纯函数，负控喂合成文本即可拆穿；不靠改坏真文件来自证。
import { NODE_KIND_HUE } from "@/components/wiki/graphViewUtils";
import { RECO_STYLE_ALIASES, RECO_STYLE_HUE } from "@/lib/stock-analysis-utils";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { type CategoricalHue, HUE_COLORS } from "../categoricalColors";

/** 剥掉注释：文档里提到某个 token 名不算使用点。 */
function stripComments(text: string): string {
  return text
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/(^|[^:])\/\/[^\n]*/g, "$1");
}

/** 定义面之一：CSS 里 `--x:` 形式的声明。 */
export function collectCssDefs(css: string): Set<string> {
  return new Set([...css.matchAll(/(--[a-zA-Z0-9-]+)\s*:/g)].map((m) => m[1]));
}

/** 定义面之二：运行时投影 `setProperty("--x", …)`（允许参数换行）。 */
export function collectProjectedDefs(ts: string): Set<string> {
  return new Set([...ts.matchAll(/setProperty\(\s*["'`]?(--[a-zA-Z0-9-]+)/g)].map((m) => m[1]));
}

/** 使用面：代码里 `var(--x)` 的字面名（以 `-` 结尾的是动态前缀，归动态判据管）。 */
export function collectVarUsages(code: string): string[] {
  return [...code.matchAll(/var\(\s*(--[a-zA-Z0-9-]+)/g)]
    .map((m) => m[1])
    .filter((n) => !n.endsWith("-"));
}

/** 动态拼接的名字（`var(--prefix-${…}`）：静态判据看不见取值域，单独登记。 */
export function collectDynamicVarPrefixes(code: string): string[] {
  return [...code.matchAll(/var\(\s*(--[a-zA-Z0-9-]*\$)/g)].map((m) => m[1].slice(0, -1));
}

/**
 * 曾经的豁免族现已清零（2026-10-28）：
 *  `--ant-*` 94 处改指应用 token —— AntD 的 ConfigProvider 没开 `cssVar`，这些名字在
 *  主应用里从来没有值；PermissionModal 拼出的 `--ant-color-<色相名>` 更是 AntD 里
 *  不存在的命名（它是语义名不是色相名），风险色条与图标因此一直不吃色。
 *  `--neon-*` 8 处则在 index.css 的别名表里补齐（取 `--cat-*` 投影，不另写十六进制），
 *  AppHeader 的流光描边与 cyberpunk / dark-neon 预设下的标题辉光因此才第一次渲染。
 */
const EXEMPT_PREFIXES: string[] = [];

function srcFiles(): string[] {
  const out: string[] = [];
  const walk = (dir: string) => {
    for (const name of readdirSync(dir)) {
      if (name === "node_modules" || name === "dist") { continue; }
      const p = join(dir, name);
      if (statSync(p).isDirectory()) { walk(p); }
      else if (/\.(tsx?|css)$/.test(name)) {
        const norm = p.replace(/\\/g, "/");
        // 判据管产出层：本门自己的字符串里必然含被检名字，排除测试自身
        if (norm.includes("/__tests__/") || /[.](test|spec)[.]tsx?$/.test(norm)) { continue; }
        out.push(p);
      }
    }
  };
  walk(join(process.cwd(), "src"));
  return out;
}

function buildDefinedSet(): Set<string> {
  const defined = new Set<string>();
  for (const f of srcFiles()) {
    // 定义面也剥注释：散文里写 `--foo: bar` 不算声明。反过来，代码里的**内嵌样式串**
    // （如 lib/exportChat.ts 生成独立 HTML 文档的 `:root{--paper:…}`）必须算声明 ——
    // 那份文档是另一个 DOM，它自己的色表就是它的权威。
    const text = stripComments(readFileSync(f, "utf8"));
    for (const n of collectCssDefs(text)) { defined.add(n); }
    for (const n of collectProjectedDefs(text)) { defined.add(n); }
  }
  // categoricalColors 用模板串投影 `--cat-${hue}`，取值域就是色板的键
  for (const hue of Object.keys(HUE_COLORS.light)) { defined.add(`--cat-${hue}`); }
  return defined;
}

describe("var() 名字必须有定义", () => {
  it("代码里不存在未定义的 var() 名字（豁免族除外）", () => {
    const defined = buildDefinedSet();
    const offenders: string[] = [];
    for (const f of srcFiles()) {
      const code = stripComments(readFileSync(f, "utf8"));
      for (const name of collectVarUsages(code)) {
        if (defined.has(name) || EXEMPT_PREFIXES.some((p) => name.startsWith(p))) { continue; }
        offenders.push(`${f.replace(/\\/g, "/")} → ${name}`);
      }
    }
    expect([...new Set(offenders)].sort()).toEqual([]);
  });

  it("负控：合成源码里的伪名字会被抓、真 token 不会", () => {
    const defined = buildDefinedSet();
    expect(defined.has("--red")).toBe(true);
    expect(defined.has("--sa-red")).toBe(false);
    expect(collectVarUsages('background: "var(--sa-red)"')).toEqual(["--sa-red"]);
    expect(collectVarUsages('background: "var(--red)"')).toEqual(["--red"]);
  });

  it("动态拼接的 var 名保持为零个未登记（新增即红，逼人来对取值域）", () => {
    const found = new Set<string>();
    for (const f of srcFiles()) {
      const code = stripComments(readFileSync(f, "utf8"));
      for (const p of collectDynamicVarPrefixes(code)) {
        if (EXEMPT_PREFIXES.some((e) => p.startsWith(e))) { continue; }
        // `--cat-${hue}` 的取值域就是色板键，由 buildDefinedSet 展开后同表核验
        if (p === "--cat-") { continue; }
        found.add(`${p} @ ${f.replace(/\\/g, "/")}`);
      }
    }
    // TaskPanel 的 `var(--color-${状态词})` 已改为显式表（三个状态词命不中 @theme 别名）
    expect([...found].sort()).toEqual([]);
  });

  it("负控：动态拼接名会被登记函数抓到", () => {
    expect(collectDynamicVarPrefixes("color: `var(--color-${cfg.color})`")).toEqual(["--color-"]);
    expect(collectDynamicVarPrefixes('color: "var(--color-success)"')).toEqual([]);
  });

  it("向后兼容别名只在 index.css 一处 authored，代码不得再用 setProperty 盖掉", () => {
    // 2026-10-28 退役的那层：App.tsx 曾把 10 个 AntD token 逐条投影到 :root 内联样式上，
    // 内联优先级高于 index.css 的同名规则 ⇒ 「向后兼容别名」表在效果上一直是空的。
    const aliased = [
      "--border-color",
      "--color-bg-container",
      "--color-bg-elevated",
      "--color-text",
      "--color-text-secondary",
      "--color-primary",
      "--color-fill-alter",
      "--table-border",
      "--hr-border-color",
      "--blockquote-border-color",
    ];
    const offenders: string[] = [];
    for (const f of srcFiles()) {
      const code = stripComments(readFileSync(f, "utf8"));
      for (const n of aliased) {
        if (code.includes(`setProperty("${n}"`)) { offenders.push(`${f.replace(/\\/g, "/")} → ${n}`); }
      }
    }
    expect(offenders).toEqual([]);
    // 负控：同一谓词对合成源码必须命中
    expect(
      `root.style.setProperty("--border-color", token.colorBorderSecondary)`.includes(`setProperty("--border-color"`),
    ).toBe(true);
  });

  it("定义面非空（防止门因为扫不到文件而假绿）", () => {
    const defined = buildDefinedSet();
    expect(defined.size).toBeGreaterThan(80);
    for (
      const n of [
        "--bg",
        "--fg",
        "--muted",
        "--red",
        "--green",
        "--amber",
        "--border-light",
        "--surface-2",
        "--red-deep",
      ]
    ) {
      expect(defined.has(n), n).toBe(true);
    }
  });
});

/** index.css 里定义了 --red 却没同时定义 --red-deep 的主题块（按块序号报）。 */
export function findUnpairedRedBlocks(css: string): number[] {
  const blocks = css.split("}");
  const bad: number[] = [];
  blocks.forEach((b, i) => {
    if (/--red\s*:/.test(b) && !/--red-deep\s*:/.test(b)) { bad.push(i); }
  });
  return bad;
}

describe("--red 与 --red-deep 成对定义", () => {
  it("index.css 每个定义 --red 的主题块都定义了 --red-deep", () => {
    const css = readFileSync(join(process.cwd(), "src/index.css"), "utf8");
    expect(findUnpairedRedBlocks(css)).toEqual([]);
    expect((css.match(/--red\s*:/g) || []).length).toBeGreaterThanOrEqual(6);
  });

  it("负控：抹掉一处 --red-deep 就应当只剩那一个块报红", () => {
    const css = readFileSync(join(process.cwd(), "src/index.css"), "utf8");
    expect(findUnpairedRedBlocks(css.replace(/--red-deep:[^;]+;/, ""))).toHaveLength(1);
  });
});

describe("类目色相登记表", () => {
  const tables: Array<[string, Record<string, string>]> = [
    ["NODE_KIND_HUE", NODE_KIND_HUE],
    ["RECO_STYLE_HUE", RECO_STYLE_HUE],
  ];

  it("登记值全部是色板里的色相", () => {
    const hues = new Set(Object.keys(HUE_COLORS.light));
    for (const [name, table] of tables) {
      for (const [k, v] of Object.entries(table)) {
        expect(hues.has(v), `${name}.${k} → ${v}`).toBe(true);
      }
    }
  });

  it("同类目表内不得撞色（撞了就等于少一档）", () => {
    for (const [name, table] of tables) {
      const values = Object.values(table);
      expect(new Set(values).size, name).toBe(values.length);
    }
  });

  it("负控：把某个风格改成与同表另一项同色，判据即红", () => {
    const mutated: Record<string, string> = { ...RECO_STYLE_HUE, value: RECO_STYLE_HUE.trend as CategoricalHue };
    expect(new Set(Object.values(mutated)).size).toBeLessThan(Object.values(mutated).length);
  });
});

/** 是否存在「第二张风格配色表」的声明（纯函数，便于负控）。 */
function hasSecondStyleTable(text: string): boolean {
  return /const STYLE_COLOR\b/.test(stripComments(text));
}

describe("风格配色只剩一处权威", () => {
  it("代码里不再有第二张 STYLE_COLOR 表", () => {
    const offenders: string[] = [];
    for (const f of srcFiles()) {
      if (hasSecondStyleTable(readFileSync(f, "utf8"))) { offenders.push(String(f)); }
    }
    expect(offenders).toEqual([]);
  });

  it("负控：合成源码里的 STYLE_COLOR 声明会被抓，RECO_STYLE_HUE 不会", () => {
    expect(hasSecondStyleTable('const STYLE_COLOR: Record<string, string> = { trend: "red" };')).toBe(true);
    expect(hasSecondStyleTable('export const RECO_STYLE_HUE = { trend: "red" };')).toBe(false);
  });

  it("别名登记必须指向表内真实存在的规范键", () => {
    for (const [alias, canonical] of Object.entries(RECO_STYLE_ALIASES)) {
      expect(RECO_STYLE_HUE[canonical], alias).toBeDefined();
    }
  });
});
