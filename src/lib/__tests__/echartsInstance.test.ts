// SPDX-License-Identifier: AGPL-3.0-only
//
// 两条判据：
//  ① 行为 —— ensureECharts 在明暗档不变时复用实例、档位翻转时销毁重建（ECharts 的坐标轴
//     文字 / tooltip 底色取自 init 时的主题，不重建就是「深字压深底」）。
//     用 vi.mock 掉 echarts：jsdom 没有 canvas 实现，真实 init 会直接抛。
//  ② 形态 —— 全仓 `echarts.init(` 只允许出现在本模块（ChartPreview 的 srcdoc 是 iframe
//     里的独立文档，另计）。扫描逻辑做成纯函数 scanRawInits(text, file)，负控喂合成源码即可拆穿。
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";

interface FakeInstance {
  theme?: string;
  disposed: boolean;
  isDisposed: () => boolean;
  dispose: () => void;
}

const created: FakeInstance[] = [];

vi.mock("echarts", () => ({
  getInstanceByDom: (el: unknown) => (el as { __inst?: FakeInstance }).__inst,
  init: (el: unknown, theme?: string) => {
    const inst: FakeInstance = {
      theme,
      disposed: false,
      isDisposed() {
        return this.disposed;
      },
      dispose() {
        this.disposed = true;
      },
    };
    created.push(inst);
    (el as { __inst?: FakeInstance }).__inst = inst;
    return inst;
  },
}));

const { ensureECharts } = await import("../echartsInstance");

function node() {
  return { clientWidth: 100, clientHeight: 100 } as unknown as HTMLElement;
}

describe("ensureECharts", () => {
  it("同一档位重复取用只 init 一次", () => {
    created.length = 0;
    const el = node();
    const a = ensureECharts(el, false);
    const b = ensureECharts(el, false);
    expect(a).toBe(b);
    expect(created.length).toBe(1);
  });

  it("明暗档翻转时销毁旧实例，并按 dark 主题重建", () => {
    created.length = 0;
    const el = node();
    const light = ensureECharts(el, false);
    const dark = ensureECharts(el, true);
    expect(light).not.toBe(dark);
    expect(light.isDisposed()).toBe(true);
    expect(created.map((i) => i.theme)).toEqual([undefined, "dark"]);
  });

  it("实例被外部 dispose 后重新取用会新建，而不是复用僵尸", () => {
    created.length = 0;
    const el = node();
    const first = ensureECharts(el, false);
    first.dispose();
    const second = ensureECharts(el, false);
    expect(second).not.toBe(first);
    expect(created.length).toBe(2);
  });
});

/** 找出「绕过 ensureECharts 直接 init」的行（纯函数，便于负控）。 */
function scanRawInits(text: string, filePath: string): string[] {
  const hits: string[] = [];
  text.split("\n").forEach((line, idx) => {
    if (!line.includes("echarts.init(")) { return; }
    // iframe 的 srcdoc 那份是独立文档（宿主 CSS 变量到不了），单独登记
    if (filePath.endsWith("ArtifactPreview/ChartPreview.tsx")) { return; }
    hits.push(`${filePath}:${idx + 1}`);
  });
  return hits;
}

function tsFiles(dir: string): string[] {
  const out: string[] = [];
  for (const name of readdirSync(dir)) {
    if (name === "node_modules" || name === "dist") { continue; }
    const p = join(dir, name);
    if (statSync(p).isDirectory()) { out.push(...tsFiles(p)); }
    else if (/\.tsx?$/.test(name)) {
      const norm = p.replace(/\\/g, "/");
      if (!norm.includes("/__tests__/") && !/[.](test|spec)[.]tsx?$/.test(norm)) { out.push(p); }
    }
  }
  return out;
}

describe("echarts.init 站点清单", () => {
  it("除 ensureECharts 内部与 iframe srcdoc 外，没有裸 echarts.init", () => {
    const offenders: string[] = [];
    for (const f of tsFiles(join(process.cwd(), "src"))) {
      const norm = f.replace(/\\/g, "/");
      if (norm.endsWith("src/lib/echartsInstance.ts")) { continue; }
      offenders.push(...scanRawInits(readFileSync(f, "utf8"), norm));
    }
    expect(offenders).toEqual([]);
  });

  it("负控：合成源码里的裸 init 会被同一函数抓出来", () => {
    expect(scanRawInits("  const c = echarts.init(el, undefined);\n", "src/pages/Fake.tsx"))
      .toEqual(["src/pages/Fake.tsx:1"]);
    expect(scanRawInits("  inst.current = ensureECharts(el, isDark);\n", "src/pages/Fake.tsx"))
      .toEqual([]);
  });
});
