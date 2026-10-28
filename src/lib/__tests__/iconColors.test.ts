// SPDX-License-Identifier: AGPL-3.0-only

import { HUE_COLORS } from "@/theme/categoricalColors";
import { describe, expect, it } from "vitest";

import {
  CHAT_ICON_COLORS,
  getIconColor,
  getIconHue,
  NAV_ICON_COLORS,
  SETTINGS_ICON_COLORS,
  TITLEBAR_ICON_COLORS,
} from "../iconColors";

const HUES = new Set(Object.keys(HUE_COLORS.light));

describe("iconColors", () => {
  describe("NAV_ICON_COLORS", () => {
    it("应包含导航栏所需的图标色相", () => {
      expect(NAV_ICON_COLORS.MessageSquare).toBe("blue");
      expect(NAV_ICON_COLORS.Sparkles).toBe("orange");
      expect(NAV_ICON_COLORS.BookOpen).toBe("emerald");
      expect(NAV_ICON_COLORS.Brain).toBe("violet");
      expect(NAV_ICON_COLORS.Link2).toBe("cyan");
      expect(NAV_ICON_COLORS.Router).toBe("volcano");
      expect(NAV_ICON_COLORS.FolderOpen).toBe("gray");
    });

    it("所有取值都必须是色板里登记过的色相", () => {
      for (const hue of Object.values(NAV_ICON_COLORS)) {
        expect(HUES.has(hue)).toBe(true);
      }
    });
  });

  describe("SETTINGS_ICON_COLORS", () => {
    it("应包含设置页图标色相", () => {
      expect(SETTINGS_ICON_COLORS.Cloud).toBe("blue");
      expect(SETTINGS_ICON_COLORS.Bot).toBe("green");
      expect(SETTINGS_ICON_COLORS.Palette).toBe("pink");
      expect(SETTINGS_ICON_COLORS.Zap).toBe("yellow");
      expect(SETTINGS_ICON_COLORS.Database).toBe("violet");
      expect(SETTINGS_ICON_COLORS.Workflow).toBe("violet");
    });

    it("所有取值都必须是色板里登记过的色相", () => {
      for (const hue of Object.values(SETTINGS_ICON_COLORS)) {
        expect(HUES.has(hue)).toBe(true);
      }
    });
  });

  describe("TITLEBAR_ICON_COLORS", () => {
    it("应包含标题栏图标色相", () => {
      expect(TITLEBAR_ICON_COLORS.Pin).toBe("blue");
      expect(TITLEBAR_ICON_COLORS.Sun).toBe("orange");
      expect(TITLEBAR_ICON_COLORS.Moon).toBe("geekblue");
      expect(TITLEBAR_ICON_COLORS.Settings).toBe("gray");
      expect(TITLEBAR_ICON_COLORS.XCircle).toBe("red");
    });

    it("所有取值都必须是色板里登记过的色相", () => {
      for (const hue of Object.values(TITLEBAR_ICON_COLORS)) {
        expect(HUES.has(hue)).toBe(true);
      }
    });
  });

  describe("CHAT_ICON_COLORS", () => {
    it("应包含常用聊天图标色相", () => {
      expect(CHAT_ICON_COLORS.Pencil).toBe("blue");
      expect(CHAT_ICON_COLORS.Copy).toBe("gray");
      expect(CHAT_ICON_COLORS.Check).toBe("green");
      expect(CHAT_ICON_COLORS.Trash2).toBe("red");
      expect(CHAT_ICON_COLORS.Bot).toBe("green");
      expect(CHAT_ICON_COLORS.Code).toBe("geekblue");
      expect(CHAT_ICON_COLORS.Mic).toBe("red");
      expect(CHAT_ICON_COLORS.Heart).toBe("red");
    });

    it("应有超过 80 个图标映射（丰富覆盖）", () => {
      expect(Object.keys(CHAT_ICON_COLORS).length).toBeGreaterThan(80);
    });

    it("所有取值都必须是色板里登记过的色相", () => {
      for (const hue of Object.values(CHAT_ICON_COLORS)) {
        expect(HUES.has(hue)).toBe(true);
      }
    });

    it("不同上下文间映射对象应各自独立", () => {
      expect(NAV_ICON_COLORS).not.toBe(SETTINGS_ICON_COLORS);
      expect(SETTINGS_ICON_COLORS).not.toBe(TITLEBAR_ICON_COLORS);
      expect(TITLEBAR_ICON_COLORS).not.toBe(CHAT_ICON_COLORS);
    });
  });

  describe("getIconColor", () => {
    it("返回 CSS 变量引用而不是固定色值（SVG 属性不解析 var，必须走 style.color）", () => {
      expect(getIconColor("Pencil")).toBe("var(--cat-blue, #1677ff)");
    });

    it('context="nav" 时应从 NAV_ICON_COLORS 查找', () => {
      expect(getIconColor("Sparkles", "nav")).toBe("var(--cat-orange, #d46b08)");
    });

    it('context="settings" 时应从 SETTINGS_ICON_COLORS 查找', () => {
      expect(getIconColor("Palette", "settings")).toBe("var(--cat-pink, #c41d7f)");
    });

    it('context="titlebar" 时应从 TITLEBAR_ICON_COLORS 查找', () => {
      expect(getIconColor("Sun", "titlebar")).toBe("var(--cat-orange, #d46b08)");
    });

    it("undefined context 应回退到 CHAT_ICON_COLORS", () => {
      expect(getIconColor("Bot", undefined)).toBe("var(--cat-green, #389e0d)");
    });

    it("指定 context 中找不到时应返回 undefined", () => {
      expect(getIconColor("Pencil", "nav")).toBeUndefined();
    });

    it("CHAT_ICON_COLORS 中也找不到时应返回 undefined", () => {
      expect(getIconColor("NonExistentIcon")).toBeUndefined();
    });
  });

  describe("getIconHue", () => {
    it("返回色相名本身，供需要实值的画布类消费方使用", () => {
      expect(getIconHue("Pencil")).toBe("blue");
      expect(getIconHue("NonExistentIcon")).toBeUndefined();
    });
  });
});
