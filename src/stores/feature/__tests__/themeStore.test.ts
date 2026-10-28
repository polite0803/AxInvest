// SPDX-License-Identifier: AGPL-3.0-only

import { applyThemeToDocument, getXtermTheme, TERMINAL_THEME_DEFAULT } from "@/stores/feature/themeStore";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/invoke", () => ({
  invoke: vi.fn(),
  listen: vi.fn(() => Promise.resolve(() => {})),
  isTauri: () => false,
  logIpcError: vi.fn(() => vi.fn()),
}));

const THEME_VARIABLES = [
  "--theme-background",
  "--theme-foreground",
  "--theme-cursor",
  "--theme-black",
  "--theme-red",
  "--theme-green",
  "--theme-yellow",
  "--theme-blue",
  "--theme-magenta",
  "--theme-cyan",
  "--theme-white",
  "--theme-bright-black",
  "--theme-bright-red",
  "--theme-bright-green",
  "--theme-bright-yellow",
  "--theme-bright-blue",
  "--theme-bright-magenta",
  "--theme-bright-cyan",
  "--theme-bright-white",
];

function resetThemeScope() {
  for (const name of THEME_VARIABLES) {
    document.documentElement.style.removeProperty(name);
  }
  document.body.style.backgroundColor = "";
  document.body.style.color = "";
}

afterEach(resetThemeScope);

describe("applyThemeToDocument", () => {
  it("把终端配色写到根节点的 --theme-* 变量", () => {
    applyThemeToDocument("monokai");

    expect(document.documentElement.style.getPropertyValue("--theme-background")).toBe("#272822");
    expect(document.documentElement.style.getPropertyValue("--theme-red")).toBe("#f92672");
    expect(document.documentElement.style.getPropertyValue("--theme-bright-cyan")).toBe("#a1efe4");
  });

  it("作用域止于根变量，不覆盖 body 的背景与前景色", () => {
    applyThemeToDocument("monokai");

    // 一旦这里被写值，整个应用的字色/底色都会被终端配色压掉（oklch 主题 token 失效）
    expect(document.body.style.backgroundColor).toBe("");
    expect(document.body.style.color).toBe("");
  });

  it("未知主题不写入任何变量", () => {
    applyThemeToDocument("does-not-exist");

    expect(document.documentElement.style.getPropertyValue("--theme-background")).toBe("");
  });
});

describe("getXtermTheme", () => {
  it("内置主题返回完整色板，含 bright 系列与 cursorAccent", () => {
    const theme = getXtermTheme("gruvbox");

    expect(theme.background).toBe("#282828");
    expect(theme.brightCyan).toBe("#8ec07c");
    expect(theme.cursorAccent).toBe("#282828");
    expect(theme.selectionBackground).toBe("#3c3836");
  });

  it("未知主题回落到终端默认色板", () => {
    expect(getXtermTheme("does-not-exist").background).toBe(TERMINAL_THEME_DEFAULT.background);
  });
});
